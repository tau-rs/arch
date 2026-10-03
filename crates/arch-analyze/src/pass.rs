//! The syntax-level pass: items from the walker, links guessed from the names the source spells
//! out, entries, routes, ports, externals and tables. Nothing here is type-checked, so every
//! link is `guessed` and says how it was guessed (ADR 0009, ADR 0010).
//!
//! How a name is resolved: a path is followed through the unit's own module tree and its `use`
//! declarations; a method call is followed only when the receiver's type is written down
//! somewhere the pass can read (a field, a parameter, a `let` with a type or a constructor).
//! What cannot be followed yields no link rather than a wrong one.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use arch_facts::{
    Access, Confidence, Framework, Item, ItemKind, Link, LinkFlags, LinkKind, Origin, Target,
    Witness,
};
use ra_ap_syntax::ast::{self, AstNode, HasArgList, HasName, HasVisibility};
use ra_ap_syntax::{Edition, SyntaxKind, SyntaxNode};

use crate::assemble::{CreatedTable, HostNote, Notes, Order, RouteNote, SpawnNote, SqlNote};
use crate::cargo::{TargetKind, UnitPlan};
use crate::items::{self, CrateCtx, Found, Ids, Lines};
use crate::sql;

mod resolved;

/// The facts of one file, before they are wrapped with its hash. What spans files is not here:
/// the file's [`Notes`] say what its bodies contribute, and [`crate::assemble`] derives it.
#[derive(Debug, Default, Clone)]
pub struct Parts {
    /// Items declared in the file.
    pub items: Vec<Item>,
    /// Links whose `from` is in the file.
    pub links: Vec<Link>,
    /// What the file's bodies contribute to facts that span files.
    pub notes: Notes,
}

/// What the pass produced: facts by repository-relative file, and which files were source files
/// of the unit.
#[derive(Debug, Default)]
pub struct Output {
    /// Facts per file.
    pub files: BTreeMap<String, Parts>,
    /// Rust files walked.
    pub rust_files: BTreeSet<String>,
    /// Rust files in the order they were walked; a file several crates walk is listed each time.
    pub walk: Vec<String>,
    /// With rust-analyzer: the walked files it has no module for, by package, whose links stay
    /// guessed (a crate added since it loaded).
    pub unresolved: BTreeMap<String, Vec<String>>,
}

const VERBS: [&str; 7] = ["get", "post", "put", "delete", "patch", "head", "options"];
const SPAWNS: [&str; 3] = ["spawn", "spawn_local", "spawn_blocking"];
const SPAWN_HOMES: [&str; 6] = ["tokio", "task", "thread", "std", "async_std", "smol"];
const PASSTHROUGH: [&str; 12] = [
    "clone",
    "as_ref",
    "as_mut",
    "borrow",
    "borrow_mut",
    "lock",
    "read",
    "write",
    "unwrap",
    "expect",
    "to_owned",
    "as_deref",
];

/// Methods every type has through std traits: on an external receiver they say nothing about
/// the external.
const STD_METHODS: [&str; 10] = [
    "to_string",
    "into",
    "try_into",
    "as_str",
    "fmt",
    "eq",
    "ne",
    "cmp",
    "hash",
    "deref",
];

struct SrcFile {
    path: String,
    krate: usize,
    lines: Lines,
    src: ast::SourceFile,
    module: Vec<String>,
}

struct Krate {
    ctx: CrateCtx,
    package: usize,
}

struct It {
    f: Found,
    file: usize,
    krate: usize,
}

#[derive(Debug, Clone)]
struct UseLeaf {
    /// `None` for a glob.
    alias: Option<String>,
    path: Vec<String>,
}

#[derive(Debug, Clone)]
enum Cur {
    Items(Vec<usize>),
    Ext { pkg: String, path: String },
    Variant(usize, String),
}

/// What a name resolved to, once one candidate is chosen.
#[derive(Debug, Clone)]
enum Tgt {
    Item(usize),
    Variant(usize, String),
    Ext { pkg: String, path: String },
}

#[derive(Debug, Clone)]
enum Ty {
    Item(usize),
    Ext { pkg: String, path: String },
}

#[derive(Debug, Clone, PartialEq)]
enum Ctx {
    Call,
    Value,
    Construct,
    Type,
    Field(String),
    ImplTrait,
    ImplSelf,
    Supertrait,
    Pattern,
    Macro,
}

#[derive(Default)]
struct Env {
    types: HashMap<String, Ty>,
    locals: HashSet<String>,
}

struct Spawn {
    order: Order,
    owner: usize,
    target: usize,
    in_loop: bool,
    witness: Witness,
}

struct SqlUse {
    owner: usize,
    touch: sql::Touch,
}

struct Route {
    order: Order,
    owner: usize,
    handler: usize,
    name: String,
    framework: Framework,
    witness: Witness,
}

struct Unit<'a> {
    plan: &'a UnitPlan,
    krates: Vec<Krate>,
    files: Vec<SrcFile>,
    items: Vec<It>,
    by_path: HashMap<String, Vec<usize>>,
    by_id: HashMap<String, usize>,
    mod_abs: HashMap<usize, String>,
    uses: HashMap<String, Vec<UseLeaf>>,
    assoc: HashMap<(usize, String), usize>,
    variants: HashMap<usize, Vec<String>>,
    fields: HashMap<(usize, String), ast::Type>,
    self_of: HashMap<usize, usize>,
    trait_impls: HashMap<usize, Vec<(usize, usize)>>,
    lib_labels: HashMap<String, String>,
    envs: HashMap<usize, Rc<Env>>,
    // results
    links: BTreeMap<(String, String, String, Option<String>), Link>,
    /// External crates' paths touched, by the file that touches them, then by package.
    touched: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    spawns: Vec<Spawn>,
    sql_uses: Vec<SqlUse>,
    routes: Vec<Route>,
    /// HTTP calls: the file they are in, where, the host, the call.
    http: Vec<(usize, Order, String, Witness)>,
    /// Direct writes to the terminal: the file, where, the call.
    tty: Vec<(usize, Order, Witness)>,
    /// Notes taken so far per file, for [`Order`].
    seq: HashMap<usize, u32>,
    /// `Cargo.lock` packages by the name code spells (`sqlx_core`): package name and line.
    lock: HashMap<String, (String, u32)>,
    /// Set while links come from the type-checked view: they are `resolved` and need no reason.
    resolved: bool,
    /// Files rust-analyzer has no module for: their links stay guessed.
    unresolved: Vec<usize>,
}

fn segs(path: &ast::Path) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = Some(path.clone());
    while let Some(p) = cur {
        out.push(p.segment()?.name_ref()?.text().to_string());
        cur = p.qualifier();
    }
    out.reverse();
    Some(out)
}

fn start(node: &SyntaxNode) -> u32 {
    node.text_range().start().into()
}

fn unquote(text: &str) -> Option<String> {
    let a = text.find('"')?;
    let b = text.rfind('"')?;
    (b > a).then(|| text[a + 1..b].to_string())
}

fn literal(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Literal(l) => unquote(&l.syntax().text().to_string()),
        _ => None,
    }
}

fn path_of(expr: &ast::Expr) -> Option<ast::Path> {
    match expr {
        ast::Expr::PathExpr(p) => p.path(),
        _ => None,
    }
}

fn ext_id(pkg: &str) -> String {
    format!("external:crate:{pkg}")
}

/// The calls of a method chain in the order they are written, and the chain's base expression.
fn chain(top: &SyntaxNode) -> (Vec<ast::MethodCallExpr>, Option<ast::Expr>) {
    let mut calls = Vec::new();
    let mut cur = ast::Expr::cast(top.clone());
    while let Some(ast::Expr::MethodCallExpr(m)) = &cur {
        let next = m.receiver();
        calls.push(m.clone());
        cur = next;
    }
    calls.reverse();
    (calls, cur)
}

fn chain_top(call: &ast::MethodCallExpr) -> SyntaxNode {
    let mut cur = call.syntax().clone();
    while let Some(parent) = cur.parent().and_then(ast::MethodCallExpr::cast) {
        if parent.receiver().is_some_and(|r| r.syntax() == &cur) {
            cur = parent.syntax().clone();
        } else {
            break;
        }
    }
    cur
}

/// A middleware's display name: the function given to `from_fn`, else the layer's type.
fn middleware_name(arg: &ast::Expr) -> Option<String> {
    for call in arg.syntax().descendants().filter_map(ast::CallExpr::cast) {
        let callee = call
            .expr()
            .as_ref()
            .and_then(path_of)
            .and_then(|p| segs(&p));
        if callee.is_some_and(|s| s.last().is_some_and(|l| l.starts_with("from_fn")))
            && let Some(last) = call.arg_list().and_then(|a| a.args().last())
            && let Some(s) = path_of(&last).and_then(|p| segs(&p))
        {
            return s.last().cloned();
        }
    }
    let s = arg
        .syntax()
        .descendants()
        .find_map(ast::Path::cast)
        .and_then(|p| segs(&p))?;
    s.iter()
        .find(|x| x.chars().next().is_some_and(char::is_uppercase))
        .or(s.last())
        .cloned()
}

impl<'a> Unit<'a> {
    fn load(root: &Path, plan: &'a UnitPlan, scope: &str) -> Result<Self> {
        let mut u = Unit {
            plan,
            krates: Vec::new(),
            files: Vec::new(),
            items: Vec::new(),
            by_path: HashMap::new(),
            by_id: HashMap::new(),
            mod_abs: HashMap::new(),
            uses: HashMap::new(),
            assoc: HashMap::new(),
            variants: HashMap::new(),
            fields: HashMap::new(),
            self_of: HashMap::new(),
            trait_impls: HashMap::new(),
            lib_labels: HashMap::new(),
            envs: HashMap::new(),
            links: BTreeMap::new(),
            touched: BTreeMap::new(),
            spawns: Vec::new(),
            sql_uses: Vec::new(),
            routes: Vec::new(),
            http: Vec::new(),
            tty: Vec::new(),
            seq: HashMap::new(),
            lock: crate::assemble::lock_packages(root),
            resolved: false,
            unresolved: Vec::new(),
        };
        for ut in &plan.unit {
            let package = &plan.packages[ut.package];
            let label = match ut.target.kind {
                TargetKind::Lib => package.ident(),
                _ => format!("{}[{}]", package.ident(), ut.target.label()),
            };
            if ut.target.kind == TargetKind::Lib {
                u.lib_labels.insert(package.ident(), label.clone());
            }
            let k = u.krates.len();
            u.krates.push(Krate {
                ctx: CrateCtx {
                    label,
                    package: package.name.clone(),
                    scope: scope.to_string(),
                    is_bin: ut.target.kind == TargetKind::Bin,
                },
                package: ut.package,
            });
            let mut ids = Ids::default();
            let mut queue: Vec<(PathBuf, Vec<String>, bool)> =
                vec![(ut.target.root.clone(), Vec::new(), true)];
            let mut seen = HashSet::new();
            while let Some((rel, module, is_root)) = queue.pop() {
                if !seen.insert(rel.clone()) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
                    continue;
                };
                let path = rel.to_string_lossy().replace('\\', "/");
                let lines = Lines::new(&text);
                let src = ast::SourceFile::parse(&text, Edition::CURRENT).tree();
                let (found, decls) = items::walk(
                    &src,
                    &u.krates[k].ctx,
                    &module,
                    &path,
                    &lines,
                    is_root,
                    &mut ids,
                );
                let fi = u.files.len();
                u.items.extend(found.into_iter().map(|f| It {
                    f,
                    file: fi,
                    krate: k,
                }));
                let is_mod_rs = is_root || rel.file_name().is_some_and(|n| n == "mod.rs");
                let dir = rel.parent().unwrap_or(Path::new("")).to_path_buf();
                let base = if is_mod_rs {
                    dir.clone()
                } else {
                    dir.join(rel.file_stem().unwrap_or_default())
                };
                for d in decls {
                    let mut child_module = d.parent.clone();
                    child_module.push(d.name.clone());
                    let child = match &d.path_attr {
                        Some(p) => Some(dir.join(p)),
                        None => {
                            let mut at = base.clone();
                            for inline in d.parent.iter().skip(module.len()) {
                                at.push(inline);
                            }
                            [
                                at.join(format!("{}.rs", d.name)),
                                at.join(&d.name).join("mod.rs"),
                            ]
                            .into_iter()
                            .find(|c| root.join(c).is_file())
                        }
                    };
                    if let Some(child) = child {
                        queue.push((child, child_module, false));
                    }
                }
                u.files.push(SrcFile {
                    path,
                    krate: k,
                    lines,
                    src,
                    module,
                });
            }
        }
        u.index();
        Ok(u)
    }

    fn label(&self, krate: usize) -> &str {
        &self.krates[krate].ctx.label
    }

    fn abs(&self, krate: usize, module: &[String]) -> String {
        std::iter::once(self.label(krate))
            .chain(module.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    }

    fn kind(&self, i: usize) -> ItemKind {
        self.items[i].f.item.kind
    }

    fn id(&self, i: usize) -> &str {
        &self.items[i].f.item.id
    }

    /// Which walk of its path file `fi` is: 0, or more when several crates walk one file.
    fn walk_of(&self, fi: usize) -> u32 {
        let path = &self.files[fi].path;
        self.files[..fi].iter().filter(|f| &f.path == path).count() as u32
    }

    /// Where the walk over file `fi` is: its next note's [`Order`].
    fn order(&mut self, fi: usize) -> Order {
        let visit = self.walk_of(fi);
        let n = self.seq.entry(fi).or_insert(0);
        *n += 1;
        [0, visit, *n]
    }

    /// The module a node owned by `owner` resolves names in.
    fn module_of(&self, owner: usize) -> String {
        self.abs(self.items[owner].krate, &self.items[owner].f.inner_module)
    }

    fn index(&mut self) {
        for (i, it) in self.items.iter().enumerate() {
            let item = &it.f.item;
            self.by_id.insert(item.id.clone(), i);
            if item.parent.is_some() || item.kind == ItemKind::Impl {
                continue;
            }
            let is_root =
                item.kind == ItemKind::Mod && item.id == format!("{}#mod", self.label(it.krate));
            let key = if is_root {
                self.label(it.krate).to_string()
            } else {
                let module: Vec<String> = if item.module.is_empty() {
                    vec![]
                } else {
                    item.module.split("::").map(str::to_string).collect()
                };
                format!("{}::{}", self.abs(it.krate, &module), item.name)
            };
            if item.kind == ItemKind::Mod {
                self.mod_abs.insert(i, key.clone());
            }
            self.by_path.entry(key).or_default().push(i);
        }
        // `use` declarations, by the module they are written in.
        for fi in 0..self.files.len() {
            let uses: Vec<ast::Use> = self.files[fi]
                .src
                .syntax()
                .descendants()
                .filter_map(ast::Use::cast)
                .collect();
            for u in uses {
                let Some(owner) = self.owner_at(fi, start(u.syntax())) else {
                    continue;
                };
                if self.kind(owner) != ItemKind::Mod {
                    continue;
                }
                let module = self.module_of(owner);
                let mut leaves = Vec::new();
                if let Some(tree) = u.use_tree() {
                    flatten(&tree, &[], &mut leaves);
                }
                self.uses.entry(module).or_default().extend(leaves);
            }
        }
        // Enum variants and struct fields: not items, but links name them (`member`).
        for i in 0..self.items.len() {
            let node = self.items[i].f.node.clone();
            if let Some(e) = ast::Enum::cast(node.clone()) {
                let names = e
                    .variant_list()
                    .into_iter()
                    .flat_map(|l| l.variants())
                    .filter_map(|v| v.name().map(|n| n.text().to_string()))
                    .collect();
                self.variants.insert(i, names);
            } else if let Some(s) = ast::Struct::cast(node) {
                match s.field_list() {
                    Some(ast::FieldList::RecordFieldList(l)) => {
                        for f in l.fields() {
                            if let (Some(n), Some(t)) = (f.name(), f.ty()) {
                                self.fields.insert((i, n.text().to_string()), t);
                            }
                        }
                    }
                    Some(ast::FieldList::TupleFieldList(l)) => {
                        for (n, f) in l.fields().enumerate() {
                            if let Some(t) = f.ty() {
                                self.fields.insert((i, n.to_string()), t);
                            }
                        }
                    }
                    None => {}
                }
            }
        }
        // Traits own their items; impls hand theirs to the type they are for.
        let mut impls: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
        for i in 0..self.items.len() {
            match self.kind(i) {
                ItemKind::Trait => {
                    self.self_of.insert(i, i);
                }
                ItemKind::Impl => {
                    let Some(imp) = ast::Impl::cast(self.items[i].f.node.clone()) else {
                        continue;
                    };
                    let module = self.module_of(i);
                    let krate = self.items[i].krate;
                    let resolve = |u: &Self, ty: Option<ast::Type>, kinds: &[ItemKind]| {
                        let path = ty?.syntax().descendants().find_map(ast::Path::cast)?;
                        match u.resolve(&module, krate, &segs(&path)?, None, 0)? {
                            Cur::Items(v) => v.into_iter().find(|x| kinds.contains(&u.kind(*x))),
                            _ => None,
                        }
                    };
                    let adt = resolve(
                        self,
                        imp.self_ty(),
                        &[ItemKind::Struct, ItemKind::Enum, ItemKind::Union],
                    );
                    let tr = resolve(self, imp.trait_(), &[ItemKind::Trait]);
                    if let Some(a) = adt {
                        self.self_of.insert(i, a);
                    }
                    impls.push((i, adt, tr));
                }
                _ => {}
            }
        }
        // Inherent impls first, so `T::name` prefers the inherent method.
        impls.sort_by_key(|(i, _, tr)| (tr.is_some(), *i));
        let children: Vec<(usize, usize)> = (0..self.items.len())
            .filter_map(|i| {
                let parent = self.items[i].f.item.parent.as_ref()?;
                Some((*self.by_id.get(parent)?, i))
            })
            .collect();
        for (parent, child) in &children {
            if self.kind(*parent) == ItemKind::Trait {
                self.self_of.insert(*child, *parent);
                self.assoc
                    .insert((*parent, self.items[*child].f.item.name.clone()), *child);
            }
        }
        for (imp, adt, tr) in impls {
            let Some(adt) = adt else { continue };
            if let Some(tr) = tr {
                self.trait_impls.entry(adt).or_default().push((imp, tr));
            }
            for (_, child) in children.iter().filter(|(p, _)| *p == imp) {
                self.self_of.insert(*child, adt);
                self.assoc
                    .entry((adt, self.items[*child].f.item.name.clone()))
                    .or_insert(*child);
            }
        }
    }

    /// The innermost item containing an offset; a file's top level belongs to its module item.
    fn owner_at(&self, fi: usize, offset: u32) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for (i, it) in self.items.iter().enumerate() {
            if it.file != fi {
                continue;
            }
            let r = it.f.node.text_range();
            let (s, e): (u32, u32) = (r.start().into(), r.end().into());
            if s <= offset && offset < e && best.is_none_or(|(len, _)| e - s <= len) {
                best = Some((e - s, i));
            }
        }
        best.map(|b| b.1).or_else(|| {
            let f = &self.files[fi];
            self.by_path
                .get(&self.abs(f.krate, &f.module))?
                .iter()
                .copied()
                .find(|i| self.kind(*i) == ItemKind::Mod)
        })
    }

    /// What `name` means inside `module`: an item declared there, or one a `use` brings in.
    /// A module and a function may share a name (`mod health; pub use health::*;`), so both
    /// sources are merged when the declared one is only a module.
    fn lookup(&self, module: &str, krate: usize, name: &str, depth: u8) -> Option<Cur> {
        if depth > 8 {
            return None;
        }
        let direct = self.by_path.get(&format!("{module}::{name}"));
        if let Some(v) = direct
            && v.iter().any(|i| self.kind(*i) != ItemKind::Mod)
        {
            return Some(Cur::Items(v.clone()));
        }
        match (direct, self.imported(module, krate, name, depth)) {
            (Some(d), Some(Cur::Items(v))) => {
                Some(Cur::Items(d.iter().copied().chain(v).collect()))
            }
            (Some(d), _) => Some(Cur::Items(d.clone())),
            (None, via) => via,
        }
    }

    fn imported(&self, module: &str, krate: usize, name: &str, depth: u8) -> Option<Cur> {
        let leaves = self.uses.get(module)?;
        if let Some(leaf) = leaves.iter().find(|l| l.alias.as_deref() == Some(name)) {
            // The leaf's first segment may be the alias's own name
            // (`use async_trait::async_trait;`), which then means the crate, not the alias.
            if leaf.path.first().is_some_and(|f| f == name) {
                let start = self.extern_crate(krate, name)?;
                return self.walk(start, &leaf.path[1..], depth + 1);
            }
            return self.resolve(module, krate, &leaf.path, None, depth + 1);
        }
        // Globs are followed a few levels deep (`pub use admin::*` over `pub use password::*`),
        // and only into the unit: a glob over an external crate would claim every unknown
        // name, std's prelude included.
        if depth >= 5 {
            return None;
        }
        for glob in leaves.iter().filter(|l| l.alias.is_none()) {
            let Some(Cur::Items(v)) = self.resolve(module, krate, &glob.path, None, depth + 1)
            else {
                continue;
            };
            for i in v {
                if let Some(m) = self.mod_abs.get(&i)
                    && m != module
                    && let Some(c) = self.lookup(m, self.items[i].krate, name, depth + 1)
                {
                    return Some(c);
                }
                if self
                    .variants
                    .get(&i)
                    .is_some_and(|vs| vs.iter().any(|x| x == name))
                {
                    return Some(Cur::Variant(i, name.to_string()));
                }
            }
        }
        None
    }

    fn extern_crate(&self, krate: usize, name: &str) -> Option<Cur> {
        if let Some(label) = self.lib_labels.get(name)
            && label != self.label(krate)
        {
            return self.by_path.get(label).map(|v| Cur::Items(v.clone()));
        }
        let package = &self.plan.packages[self.krates[krate].package];
        let dep = package.deps.iter().find(|d| d.ident == name)?;
        Some(Cur::Ext {
            pkg: dep.package.clone(),
            path: name.to_string(),
        })
    }

    /// Follow a written path from a module. `self_idx` is what `Self` means there.
    fn resolve(
        &self,
        module: &str,
        krate: usize,
        path: &[String],
        self_idx: Option<usize>,
        depth: u8,
    ) -> Option<Cur> {
        let first = path.first()?;
        let (cur, i) = match first.as_str() {
            "crate" => (Cur::Items(self.by_path.get(self.label(krate))?.clone()), 1),
            "self" => (Cur::Items(self.by_path.get(module)?.clone()), 1),
            "super" => {
                let mut m = module;
                let mut n = 0;
                while path.get(n).is_some_and(|s| s == "super") {
                    m = m.rsplit_once("::")?.0;
                    n += 1;
                }
                (Cur::Items(self.by_path.get(m)?.clone()), n)
            }
            "Self" => (Cur::Items(vec![self_idx?]), 1),
            name => (
                self.lookup(module, krate, name, depth)
                    .or_else(|| self.extern_crate(krate, name))?,
                1,
            ),
        };
        self.walk(cur, &path[i..], depth)
    }

    /// Follow the rest of a path from where its start resolved to.
    fn walk(&self, mut cur: Cur, rest: &[String], depth: u8) -> Option<Cur> {
        for seg in rest {
            cur = match cur {
                Cur::Items(v) => {
                    if let Some(m) = v.iter().find_map(|x| self.mod_abs.get(x)) {
                        let k = self.items[v[0]].krate;
                        self.lookup(m, k, seg, depth)?
                    } else {
                        let t = *v.iter().find(|x| {
                            matches!(
                                self.kind(**x),
                                ItemKind::Struct
                                    | ItemKind::Enum
                                    | ItemKind::Union
                                    | ItemKind::Trait
                            )
                        })?;
                        if self.variants.get(&t).is_some_and(|vs| vs.contains(seg)) {
                            Cur::Variant(t, seg.clone())
                        } else {
                            Cur::Items(vec![*self.assoc.get(&(t, seg.clone()))?])
                        }
                    }
                }
                Cur::Ext { pkg, path } => Cur::Ext {
                    pkg,
                    path: format!("{path}::{seg}"),
                },
                Cur::Variant(..) => return None,
            };
        }
        Some(cur)
    }

    /// The type a written type stands for, as far as the pass can tell: the first type of the
    /// unit named in it (`Arc<dyn OrderRepository>` → the trait), else the first external one.
    fn ty_of(
        &self,
        ty: &ast::Type,
        module: &str,
        krate: usize,
        self_idx: Option<usize>,
    ) -> Option<Ty> {
        let mut ext = None;
        for p in ty.syntax().descendants().filter_map(ast::Path::cast) {
            if p.syntax()
                .parent()
                .is_none_or(|x| x.kind() != SyntaxKind::PATH_TYPE)
            {
                continue;
            }
            let Some(s) = segs(&p) else { continue };
            match self.resolve(module, krate, &s, self_idx, 0) {
                Some(Cur::Items(v)) => {
                    if let Some(t) = v.into_iter().find(|x| {
                        matches!(
                            self.kind(*x),
                            ItemKind::Struct | ItemKind::Enum | ItemKind::Union | ItemKind::Trait
                        )
                    }) {
                        return Some(Ty::Item(t));
                    }
                }
                Some(Cur::Ext { pkg, path }) if ext.is_none() && path.contains("::") => {
                    ext = Some(Ty::Ext { pkg, path })
                }
                _ => {}
            }
        }
        ext
    }

    fn env(&mut self, owner: usize) -> Rc<Env> {
        if let Some(e) = self.envs.get(&owner) {
            return e.clone();
        }
        let mut env = Env::default();
        if let Some(f) = ast::Fn::cast(self.items[owner].f.node.clone()) {
            let module = self.module_of(owner);
            let krate = self.items[owner].krate;
            let self_idx = self.self_of.get(&owner).copied();
            let bind = |pat: Option<ast::Pat>| -> Option<String> {
                let pat = pat?;
                pat.syntax()
                    .descendants()
                    .find_map(ast::IdentPat::cast)?
                    .name()
                    .map(|n| n.text().to_string())
            };
            for p in f.param_list().into_iter().flat_map(|l| l.params()) {
                if let (Some(name), Some(ty)) = (bind(p.pat()), p.ty())
                    && let Some(t) = self.ty_of(&ty, &module, krate, self_idx)
                {
                    env.types.insert(name, t);
                }
            }
            for pat in f.syntax().descendants().filter_map(ast::IdentPat::cast) {
                if let Some(n) = pat.name() {
                    env.locals.insert(n.text().to_string());
                }
            }
            for l in f.syntax().descendants().filter_map(ast::LetStmt::cast) {
                let Some(name) = bind(l.pat()) else { continue };
                let t = match l.ty() {
                    Some(ty) => self.ty_of(&ty, &module, krate, self_idx),
                    None => l.initializer().and_then(|e| self.type_of(&e, owner, &env)),
                };
                if let Some(t) = t {
                    env.types.insert(name, t);
                }
            }
        }
        let env = Rc::new(env);
        self.envs.insert(owner, env.clone());
        env
    }

    fn method_on(&self, ty: usize, name: &str) -> Option<usize> {
        self.assoc.get(&(ty, name.to_string())).copied()
    }

    fn ret_ty(&self, f: usize) -> Option<Ty> {
        let node = ast::Fn::cast(self.items[f].f.node.clone())?;
        let ty = node.ret_type()?.ty()?;
        self.ty_of(
            &ty,
            &self.module_of(f),
            self.items[f].krate,
            self.self_of.get(&f).copied(),
        )
    }

    fn type_of(&self, expr: &ast::Expr, owner: usize, env: &Env) -> Option<Ty> {
        let module = self.module_of(owner);
        let krate = self.items[owner].krate;
        let self_idx = self.self_of.get(&owner).copied();
        match expr {
            ast::Expr::PathExpr(p) => {
                let s = segs(&p.path()?)?;
                if s.len() == 1 {
                    if s[0] == "self" {
                        return self_idx.map(Ty::Item);
                    }
                    if let Some(t) = env.types.get(&s[0]) {
                        return Some(t.clone());
                    }
                }
                match self.resolve(&module, krate, &s, self_idx, 0)? {
                    Cur::Items(v) => v
                        .into_iter()
                        .find(|x| matches!(self.kind(*x), ItemKind::Struct | ItemKind::Union))
                        .map(Ty::Item),
                    Cur::Variant(e, _) => Some(Ty::Item(e)),
                    Cur::Ext { .. } => None,
                }
            }
            ast::Expr::FieldExpr(f) => {
                let Ty::Item(t) = self.type_of(&f.expr()?, owner, env)? else {
                    return None;
                };
                let ty = self.fields.get(&(t, f.name_ref()?.text().to_string()))?;
                self.ty_of(ty, &self.module_of(t), self.items[t].krate, Some(t))
            }
            ast::Expr::MethodCallExpr(m) => {
                let recv = self.type_of(&m.receiver()?, owner, env)?;
                let name = m.name_ref()?.text().to_string();
                if PASSTHROUGH.contains(&name.as_str()) {
                    return Some(recv);
                }
                match recv {
                    Ty::Item(t) => self.ret_ty(self.method_on(t, &name)?),
                    Ty::Ext { .. } => None,
                }
            }
            ast::Expr::CallExpr(c) => {
                let s = segs(&path_of(&c.expr()?)?)?;
                match self.resolve(&module, krate, &s, self_idx, 0)? {
                    Cur::Items(v) => {
                        if let Some(f) = v.iter().copied().find(|x| self.kind(*x) == ItemKind::Fn) {
                            self.ret_ty(f)
                        } else {
                            v.into_iter()
                                .find(|x| self.kind(*x) == ItemKind::Struct)
                                .map(Ty::Item)
                        }
                    }
                    Cur::Variant(e, _) => Some(Ty::Item(e)),
                    Cur::Ext { pkg, path } => {
                        // `reqwest::Client::new()` is a `reqwest::Client`.
                        let (ty, f) = path.rsplit_once("::")?;
                        (f == "new" || f == "default" || f == "builder")
                            .then(|| Ty::Ext {
                                pkg,
                                path: ty.to_string(),
                            })
                            .filter(|t| matches!(t, Ty::Ext { path, .. } if path.contains("::")))
                    }
                }
            }
            ast::Expr::RecordExpr(r) => {
                match self.resolve(&module, krate, &segs(&r.path()?)?, self_idx, 0)? {
                    Cur::Items(v) => v
                        .into_iter()
                        .find(|x| self.kind(*x) == ItemKind::Struct)
                        .map(Ty::Item),
                    Cur::Variant(e, _) => Some(Ty::Item(e)),
                    Cur::Ext { .. } => None,
                }
            }
            ast::Expr::AwaitExpr(e) => self.type_of(&e.expr()?, owner, env),
            ast::Expr::TryExpr(e) => self.type_of(&e.expr()?, owner, env),
            ast::Expr::ParenExpr(e) => self.type_of(&e.expr()?, owner, env),
            ast::Expr::RefExpr(e) => self.type_of(&e.expr()?, owner, env),
            ast::Expr::PrefixExpr(e) => self.type_of(&e.expr()?, owner, env),
            _ => None,
        }
    }

    fn witness(&self, fi: usize, offset: u32) -> Witness {
        let (line, col) = self.files[fi].lines.pos(offset);
        Witness::Span {
            file: self.files[fi].path.clone(),
            line,
            col: Some(col),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn link(
        &mut self,
        from: usize,
        to: Target,
        kind: LinkKind,
        member: Option<String>,
        witness: Witness,
        reason: &str,
        access: Option<Access>,
    ) {
        let origin = matches!(
            kind,
            LinkKind::Implements
                | LinkKind::Refines
                | LinkKind::ReExports
                | LinkKind::Inherits
                | LinkKind::Decorates
                | LinkKind::Holds
        )
        .then_some(Origin::Declared);
        let compile_time = matches!(
            kind,
            LinkKind::Expands | LinkKind::Decorates | LinkKind::Inherits
        );
        let from = self.id(from).to_string();
        let key = (
            from.clone(),
            format!("{to:?}"),
            format!("{kind:?}"),
            member.clone(),
        );
        self.links.entry(key).or_insert(Link {
            from,
            to,
            kind,
            confidence: if self.resolved {
                Confidence::Resolved
            } else {
                Confidence::Guessed
            },
            witness,
            flags: LinkFlags {
                origin,
                access,
                compile_time,
                r#unsafe: false,
            },
            reason: (!self.resolved).then(|| reason.to_string()),
            member,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn external(
        &mut self,
        from: usize,
        pkg: &str,
        path: &str,
        kind: LinkKind,
        member: Option<String>,
        w: Witness,
        why: &str,
    ) {
        let file = self.items[from].f.item.file.clone();
        self.touched
            .entry(file)
            .or_default()
            .entry(pkg.to_string())
            .or_default()
            .insert(path.to_string());
        self.link(
            from,
            Target::External(ext_id(pkg)),
            kind,
            member,
            w,
            why,
            None,
        );
    }

    fn visit_file(&mut self, fi: usize) {
        let root = self.files[fi].src.syntax().clone();
        self.visit(fi, &root, 0, 0, None, 0);
    }

    /// Visit a tree. For a re-parsed macro body, `base` and `sub` map its offsets back into the
    /// file and `fixed` is the item the macro call sits in.
    fn visit(
        &mut self,
        fi: usize,
        root: &SyntaxNode,
        base: u32,
        sub: u32,
        fixed: Option<usize>,
        depth: u8,
    ) {
        for node in root.descendants() {
            let kind = node.kind();
            if !matches!(
                kind,
                SyntaxKind::PATH
                    | SyntaxKind::METHOD_CALL_EXPR
                    | SyntaxKind::FIELD_EXPR
                    | SyntaxKind::MACRO_CALL
                    | SyntaxKind::ATTR
                    | SyntaxKind::USE
            ) {
                continue;
            }
            let at = start(&node).saturating_sub(sub) + base;
            let Some(owner) = fixed.or_else(|| self.owner_at(fi, at)) else {
                continue;
            };
            let pos = |n: &SyntaxNode| start(n).saturating_sub(sub) + base;
            match kind {
                SyntaxKind::PATH => {
                    if node.parent().is_some_and(|p| p.kind() == SyntaxKind::PATH) {
                        continue;
                    }
                    if let Some(p) = ast::Path::cast(node.clone()) {
                        let at = p.segment().map_or(at, |s| pos(s.syntax()));
                        self.on_path(fi, &p, owner, at);
                    }
                }
                SyntaxKind::METHOD_CALL_EXPR => {
                    if let Some(m) = ast::MethodCallExpr::cast(node.clone()) {
                        let at = m.name_ref().map_or(at, |n| pos(n.syntax()));
                        self.on_method(fi, &m, owner, at);
                    }
                }
                SyntaxKind::FIELD_EXPR => {
                    if let Some(f) = ast::FieldExpr::cast(node.clone()) {
                        let at = f.name_ref().map_or(at, |n| pos(n.syntax()));
                        self.on_field(fi, &f, owner, at);
                    }
                }
                SyntaxKind::MACRO_CALL => {
                    if let Some(m) = ast::MacroCall::cast(node.clone()) {
                        self.on_macro(fi, &m, owner, at, base, sub, depth);
                    }
                }
                SyntaxKind::ATTR => self.on_attr(fi, &node, owner, at),
                SyntaxKind::USE if fixed.is_none() => {
                    if let Some(u) = ast::Use::cast(node.clone()) {
                        self.on_use(fi, &u, owner, at);
                    }
                }
                _ => {}
            }
        }
    }

    fn ctx_of(&self, path: &ast::Path) -> Option<(Ctx, SyntaxNode)> {
        let parent = path.syntax().parent()?;
        Some(match parent.kind() {
            SyntaxKind::PATH_EXPR => {
                let call = parent.parent().and_then(ast::CallExpr::cast);
                match call {
                    Some(c) if c.expr().is_some_and(|e| e.syntax() == &parent) => {
                        (Ctx::Call, c.syntax().clone())
                    }
                    _ => (Ctx::Value, parent),
                }
            }
            SyntaxKind::RECORD_EXPR => (Ctx::Construct, parent),
            SyntaxKind::PATH_PAT | SyntaxKind::TUPLE_STRUCT_PAT | SyntaxKind::RECORD_PAT => {
                (Ctx::Pattern, parent)
            }
            SyntaxKind::MACRO_CALL => (Ctx::Macro, parent),
            SyntaxKind::PATH_TYPE => {
                if let Some(imp) = parent.parent().and_then(ast::Impl::cast) {
                    let is = |t: Option<ast::Type>| t.is_some_and(|t| t.syntax() == &parent);
                    if imp.trait_().is_some() && is(imp.trait_()) {
                        return Some((Ctx::ImplTrait, parent));
                    }
                    if is(imp.self_ty()) {
                        return Some((Ctx::ImplSelf, parent));
                    }
                }
                for a in parent.ancestors().skip(1) {
                    match a.kind() {
                        SyntaxKind::TYPE_BOUND_LIST
                            if a.parent().is_some_and(|p| p.kind() == SyntaxKind::TRAIT) =>
                        {
                            return Some((Ctx::Supertrait, parent));
                        }
                        SyntaxKind::RECORD_FIELD => {
                            let name = ast::RecordField::cast(a)?.name()?.text().to_string();
                            return Some((Ctx::Field(name), parent));
                        }
                        SyntaxKind::TUPLE_FIELD => {
                            let n = std::iter::successors(a.prev_sibling(), |s| s.prev_sibling())
                                .filter(|s| s.kind() == SyntaxKind::TUPLE_FIELD)
                                .count();
                            return Some((Ctx::Field(n.to_string()), parent));
                        }
                        SyntaxKind::FN
                        | SyntaxKind::IMPL
                        | SyntaxKind::TRAIT
                        | SyntaxKind::STRUCT
                        | SyntaxKind::ENUM => {
                            break;
                        }
                        _ => {}
                    }
                }
                (Ctx::Type, parent)
            }
            _ => return None,
        })
    }

    fn pick(&self, v: &[usize], ctx: &Ctx) -> Option<usize> {
        use ItemKind::*;
        let order: &[ItemKind] = match ctx {
            Ctx::Call | Ctx::Value => &[Fn, Const, Static, Struct, Union],
            Ctx::Construct | Ctx::Pattern => &[Struct, Union, Enum],
            Ctx::Macro => &[Macro],
            _ => &[Struct, Enum, Union, Trait, TypeAlias],
        };
        order
            .iter()
            .find_map(|k| v.iter().copied().find(|i| self.kind(*i) == *k))
    }

    fn on_path(&mut self, fi: usize, path: &ast::Path, owner: usize, at: u32) {
        let Some((ctx, expr)) = self.ctx_of(path) else {
            return;
        };
        let Some(s) = segs(path) else { return };
        let krate = self.items[owner].krate;
        let module = self.module_of(owner);
        let self_idx = self.self_of.get(&owner).copied();
        if ctx == Ctx::Value && s.len() == 1 && self.env(owner).locals.contains(&s[0]) {
            return;
        }
        let Some(cur) = self.resolve(&module, krate, &s, self_idx, 0) else {
            return;
        };
        let w = self.witness(fi, at);
        let tgt = match &cur {
            Cur::Items(v) => self.pick(v, &ctx).map(Tgt::Item),
            Cur::Variant(e, name) => Some(Tgt::Variant(*e, name.clone())),
            Cur::Ext { pkg, path } => Some(Tgt::Ext {
                pkg: pkg.clone(),
                path: path.clone(),
            }),
        };
        if let Some(tgt) = tgt {
            self.emit(owner, tgt, &ctx, &expr, w.clone());
        }
        if ctx == Ctx::Call
            && let Some(call) = ast::CallExpr::cast(expr)
        {
            self.on_call(fi, &call, &s, &cur, owner, w);
        }
    }

    /// The link a name gives in a context: the one table both passes share. `expr` is the
    /// expression or type node the name sits in.
    fn emit(&mut self, owner: usize, tgt: Tgt, ctx: &Ctx, expr: &SyntaxNode, w: Witness) {
        let why = "path followed through modules and `use`";
        let is_test = self.items[owner].f.is_test;
        let self_idx = self.self_of.get(&owner).copied();
        match tgt {
            Tgt::Item(t) => {
                let to = Target::Item(self.id(t).to_string());
                let (kind, member, access) = match (self.kind(t), ctx) {
                    (ItemKind::Fn, Ctx::Call) => {
                        if is_spawn_arg(expr) {
                            // `spawn(f(..))` hands `f` off (see `on_call`); it is not a direct call.
                            return;
                        }
                        let port = self
                            .self_of
                            .get(&t)
                            .is_some_and(|p| self.kind(*p) == ItemKind::Trait);
                        let kind = if is_test {
                            LinkKind::Tests
                        } else if port {
                            LinkKind::CallsPort
                        } else {
                            LinkKind::Calls
                        };
                        (kind, None, None)
                    }
                    (ItemKind::Fn, Ctx::Value) => (LinkKind::RefersTo, None, None),
                    (ItemKind::Const | ItemKind::Static, Ctx::Call | Ctx::Value) => {
                        (LinkKind::Reads, None, Some(Access::Read))
                    }
                    (
                        ItemKind::Struct | ItemKind::Union,
                        Ctx::Call | Ctx::Construct | Ctx::Value,
                    ) => (LinkKind::Constructs, None, None),
                    (
                        ItemKind::Struct | ItemKind::Union | ItemKind::Enum | ItemKind::TypeAlias,
                        Ctx::Type,
                    ) => {
                        if self_idx == Some(t) {
                            return;
                        }
                        (LinkKind::UsesType, None, None)
                    }
                    (ItemKind::Struct | ItemKind::Union | ItemKind::Enum, Ctx::ImplSelf) => {
                        (LinkKind::UsesType, None, None)
                    }
                    (
                        ItemKind::Struct | ItemKind::Union | ItemKind::Enum | ItemKind::TypeAlias,
                        Ctx::Field(f),
                    ) => (LinkKind::Holds, Some(f.clone()), None),
                    (ItemKind::Trait, Ctx::ImplTrait) => (LinkKind::Implements, None, None),
                    (ItemKind::Trait, Ctx::Supertrait) => (LinkKind::Refines, None, None),
                    (ItemKind::Trait, Ctx::Type | Ctx::Field(_)) => {
                        (LinkKind::DependsOnPort, None, None)
                    }
                    (ItemKind::Macro, Ctx::Macro) => (LinkKind::Expands, None, None),
                    _ => return,
                };
                self.link(owner, to, kind, member, w.clone(), why, access);
                if matches!(ctx, Ctx::Call | Ctx::Construct | Ctx::Value) {
                    let made = match self.kind(t) {
                        ItemKind::Struct => Some(t),
                        ItemKind::Fn => self
                            .self_of
                            .get(&t)
                            .copied()
                            .filter(|a| self.kind(*a) == ItemKind::Struct),
                        _ => None,
                    };
                    if let Some(made) = made {
                        self.wires(owner, made, expr, w);
                    }
                }
            }
            Tgt::Variant(e, name) => {
                let kind = match ctx {
                    Ctx::Call | Ctx::Construct | Ctx::Value => LinkKind::Constructs,
                    Ctx::Pattern => LinkKind::MatchesOn,
                    _ => return,
                };
                let to = Target::Item(self.id(e).to_string());
                self.link(owner, to, kind, Some(name), w, why, None);
            }
            Tgt::Ext { pkg, path: p } => {
                if !p.contains("::") {
                    return;
                }
                let (kind, member) = match ctx {
                    Ctx::Call => (LinkKind::CallsOut, None),
                    Ctx::Value => (LinkKind::RefersTo, None),
                    Ctx::Construct => (LinkKind::Constructs, None),
                    Ctx::Type | Ctx::ImplSelf => (LinkKind::UsesType, None),
                    Ctx::Field(f) => (LinkKind::Holds, Some(f.clone())),
                    Ctx::ImplTrait => (LinkKind::Implements, None),
                    Ctx::Supertrait => (LinkKind::Refines, None),
                    Ctx::Macro => (LinkKind::Expands, None),
                    Ctx::Pattern => return,
                };
                let shown = if *ctx == Ctx::Macro {
                    format!("{p}!")
                } else {
                    p
                };
                self.external(owner, &pkg, &shown, kind, member, w, why);
            }
        }
    }

    /// `wires`: a value of a type that implements one of the unit's traits is built and passed
    /// on as an argument, which is what composition code does.
    fn wires(&mut self, owner: usize, made: usize, expr: &SyntaxNode, w: Witness) {
        if self.self_of.get(&owner) == Some(&made) {
            return;
        }
        let passed = expr
            .ancestors()
            .skip(1)
            .take_while(|a| a.kind() != SyntaxKind::FN)
            .any(|a| a.kind() == SyntaxKind::ARG_LIST);
        if !passed {
            return;
        }
        // A pattern, not a type-checked fact: it stays guessed in both passes.
        let was = std::mem::replace(&mut self.resolved, false);
        for (imp, tr) in self.trait_impls.get(&made).cloned().unwrap_or_default() {
            let why = format!(
                "builds a `{}` and passes it on; it implements `{}`",
                self.items[made].f.item.name, self.items[tr].f.item.name
            );
            self.link(
                owner,
                Target::Item(self.id(imp).to_string()),
                LinkKind::Wires,
                None,
                w.clone(),
                &why,
                None,
            );
        }
        self.resolved = was;
    }

    fn on_call(
        &mut self,
        fi: usize,
        call: &ast::CallExpr,
        s: &[String],
        cur: &Cur,
        owner: usize,
        w: Witness,
    ) {
        let last = s.last().map(String::as_str).unwrap_or("");
        let arg0 = call.arg_list().and_then(|a| a.args().next());
        // spawn(f(..)): work handed to another task (ADR 0028 §1).
        if SPAWNS.contains(&last)
            && s.len() > 1
            && s[..s.len() - 1]
                .iter()
                .any(|x| SPAWN_HOMES.contains(&x.as_str()))
        {
            let mut e = arg0.clone();
            for _ in 0..4 {
                e = match e {
                    Some(ast::Expr::ClosureExpr(c)) => c.body(),
                    Some(ast::Expr::BlockExpr(b)) => b.stmt_list().and_then(|l| l.tail_expr()),
                    Some(ast::Expr::AwaitExpr(a)) => a.expr(),
                    other => {
                        e = other;
                        break;
                    }
                };
            }
            if let Some(ast::Expr::CallExpr(inner)) = e
                && let Some(p) = inner
                    .expr()
                    .as_ref()
                    .and_then(path_of)
                    .and_then(|p| segs(&p))
                && let Some(Cur::Items(v)) = self.resolve(
                    &self.module_of(owner),
                    self.items[owner].krate,
                    &p,
                    self.self_of.get(&owner).copied(),
                    0,
                )
                && let Some(target) = v.into_iter().find(|x| self.kind(*x) == ItemKind::Fn)
            {
                let in_loop = call
                    .syntax()
                    .ancestors()
                    .take_while(|a| a.kind() != SyntaxKind::FN)
                    .any(|a| {
                        matches!(
                            a.kind(),
                            SyntaxKind::LOOP_EXPR | SyntaxKind::WHILE_EXPR | SyntaxKind::FOR_EXPR
                        )
                    });
                let to = Target::Item(self.id(target).to_string());
                self.link(
                    owner,
                    to,
                    LinkKind::HandsOff,
                    None,
                    w.clone(),
                    &s.join("::"),
                    None,
                );
                let order = self.order(fi);
                self.spawns.push(Spawn {
                    order,
                    owner,
                    target,
                    in_loop,
                    witness: w.clone(),
                });
            }
        }
        // sqlx::query("…"): the tables the statement touches.
        if let Cur::Ext { pkg, .. } = cur
            && pkg.starts_with("sqlx")
            && last.starts_with("query")
            && let Some(text) = arg0.as_ref().and_then(literal)
        {
            let at = arg0.map_or(0, |a| start(a.syntax()));
            let w = self.witness(fi, at);
            self.on_sql(owner, &text, w, &s.join("::"));
        }
    }

    fn on_sql(&mut self, owner: usize, text: &str, w: Witness, via: &str) {
        for touch in sql::touches(text) {
            let why = format!("SQL text given to `{via}`");
            let to = Target::Table(touch.table.clone());
            self.link(
                owner,
                to.clone(),
                LinkKind::Reads,
                None,
                w.clone(),
                &why,
                Some(touch.access.into()),
            );
            if touch.dequeues {
                let why = format!("{why}: `FOR UPDATE SKIP LOCKED` claims rows");
                self.link(
                    owner,
                    to,
                    LinkKind::Queues,
                    None,
                    w.clone(),
                    &why,
                    Some(Access::Read),
                );
            }
            self.sql_uses.push(SqlUse { owner, touch });
        }
    }

    fn on_method(&mut self, fi: usize, m: &ast::MethodCallExpr, owner: usize, at: u32) {
        let Some(name) = m.name_ref().map(|n| n.text().to_string()) else {
            return;
        };
        if name == "route" {
            self.on_route(fi, m, owner, at);
        }
        let env = self.env(owner);
        let Some(recv) = m.receiver().and_then(|r| self.type_of(&r, owner, &env)) else {
            return;
        };
        let w = self.witness(fi, at);
        match recv {
            Ty::Item(t) => {
                let Some(f) = self.method_on(t, &name) else {
                    return;
                };
                let port = self
                    .self_of
                    .get(&f)
                    .is_some_and(|p| self.kind(*p) == ItemKind::Trait);
                let kind = if self.items[owner].f.is_test {
                    LinkKind::Tests
                } else if port {
                    LinkKind::CallsPort
                } else {
                    LinkKind::Calls
                };
                let why = format!(
                    "method on `{}`, the receiver's written type",
                    self.items[t].f.item.name
                );
                self.link(
                    owner,
                    Target::Item(self.id(f).to_string()),
                    kind,
                    None,
                    w,
                    &why,
                    None,
                );
            }
            Ty::Ext { pkg, path } => {
                if PASSTHROUGH.contains(&name.as_str()) || STD_METHODS.contains(&name.as_str()) {
                    return;
                }
                let why = format!("method on `{path}`, the receiver's written type");
                self.external(
                    owner,
                    &pkg,
                    &format!("{path}::{name}"),
                    LinkKind::CallsOut,
                    None,
                    w.clone(),
                    &why,
                );
                if pkg == "reqwest" && VERBS.contains(&name.as_str()) {
                    let f = &self.files[fi];
                    let host = host_in(f.lines.text());
                    let (host, how) = match host {
                        Some(h) => (h, "host from a literal in the file"),
                        None => {
                            let m = f
                                .module
                                .last()
                                .cloned()
                                .unwrap_or_else(|| self.krates[f.krate].ctx.package.clone());
                            (m, "host is not in the source; named after the module")
                        }
                    };
                    let id = format!("external:http:{host}");
                    let why = format!("`reqwest` {name} call; {how}");
                    self.link(
                        owner,
                        Target::External(id.clone()),
                        LinkKind::CallsOut,
                        None,
                        w.clone(),
                        &why,
                        None,
                    );
                    let order = self.order(fi);
                    self.http.push((fi, order, host, w));
                }
            }
        }
    }

    fn on_field(&mut self, fi: usize, f: &ast::FieldExpr, owner: usize, at: u32) {
        let Some(name) = f.name_ref().map(|n| n.text().to_string()) else {
            return;
        };
        let env = self.env(owner);
        let Some(Ty::Item(t)) = f.expr().and_then(|e| self.type_of(&e, owner, &env)) else {
            return;
        };
        if !self.fields.contains_key(&(t, name.clone())) || self.self_of.get(&owner) == Some(&t) {
            return;
        }
        let writes = f
            .syntax()
            .parent()
            .and_then(ast::BinExpr::cast)
            .is_some_and(|b| {
                let assigns = b.op_token().is_some_and(|t| {
                    let t = t.text();
                    t.ends_with('=') && !matches!(t, "==" | "!=" | "<=" | ">=")
                });
                assigns && b.lhs().is_some_and(|l| l.syntax() == f.syntax())
            });
        let access = if writes { Access::Write } else { Access::Read };
        let w = self.witness(fi, at);
        let why = format!(
            "field of `{}`, the receiver's written type",
            self.items[t].f.item.name
        );
        self.link(
            owner,
            Target::Item(self.id(t).to_string()),
            LinkKind::Reads,
            Some(name),
            w,
            &why,
            Some(access),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn on_macro(
        &mut self,
        fi: usize,
        m: &ast::MacroCall,
        owner: usize,
        at: u32,
        base: u32,
        sub: u32,
        depth: u8,
    ) {
        let Some(tt) = m.token_tree() else { return };
        let s = m.path().and_then(|p| segs(&p)).unwrap_or_default();
        let last = s.last().map(String::as_str).unwrap_or("");
        // Only a direct write to the terminal is the tty external. Logging through a facade
        // (`tracing`, `log`) is a library call, linked to its crate like any other macro
        // (arch-design#28).
        let writes =
            s.len() == 1 && ["println", "eprintln", "print", "eprint", "dbg"].contains(&last);
        if writes {
            let w = self.witness(fi, at);
            let why = format!("`{}!` writes to the terminal", s.join("::"));
            self.link(
                owner,
                Target::External("external:tty:log".into()),
                LinkKind::CallsOut,
                None,
                w.clone(),
                &why,
                None,
            );
            let order = self.order(fi);
            self.tty.push((fi, order, w));
        }
        let text = tt.syntax().text().to_string();
        if s.first().is_some_and(|x| x == "sqlx")
            && last.starts_with("query")
            && let Some(a) = text.find('"')
            && let Some(len) = text[a + 1..].find('"')
        {
            let w = self.witness(fi, start(tt.syntax()).saturating_sub(sub) + base + a as u32);
            self.on_sql(
                owner,
                &text[a + 1..a + 1 + len],
                w,
                &format!("{}!", s.join("::")),
            );
        }
        // A macro's arguments are tokens, not syntax. Most are expressions in practice, so they
        // are read again as a function body; what does not parse yields nothing.
        if depth < 3 && text.len() > 2 && text.len() < 20_000 {
            let prefix = "fn f(){";
            let inner = &text[1..text.len() - 1];
            let parsed =
                ast::SourceFile::parse(&format!("{prefix}{inner}}}"), Edition::CURRENT).tree();
            let new_base = start(tt.syntax()).saturating_sub(sub) + base + 1;
            self.visit(
                fi,
                parsed.syntax(),
                new_base,
                prefix.len() as u32,
                Some(owner),
                depth + 1,
            );
        }
    }

    fn on_attr(&mut self, fi: usize, attr: &SyntaxNode, owner: usize, at: u32) {
        let text: String = attr.text().to_string().split_whitespace().collect();
        let Some(body) = text.strip_prefix("#[").and_then(|t| t.strip_suffix(']')) else {
            return;
        };
        let krate = self.items[owner].krate;
        // Attributes sit on the item; names in them resolve in the module around it.
        let module = match self.kind(owner) {
            ItemKind::Mod => self.abs(
                krate,
                &self.items[owner]
                    .f
                    .item
                    .module
                    .split("::")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            ),
            _ => self.module_of(owner),
        };
        let w = self.witness(fi, at);
        let (names, kind): (Vec<&str>, LinkKind) = match body
            .strip_prefix("derive(")
            .and_then(|d| d.strip_suffix(')'))
        {
            Some(list) => (
                list.split(',').filter(|n| !n.is_empty()).collect(),
                LinkKind::Inherits,
            ),
            None => (
                vec![body.split('(').next().unwrap_or(body)],
                LinkKind::Decorates,
            ),
        };
        for name in names {
            let path: Vec<String> = name.split("::").map(str::to_string).collect();
            match self.resolve(&module, krate, &path, None, 0) {
                Some(Cur::Ext { pkg, path }) if path.contains("::") => {
                    let why = if kind == LinkKind::Inherits {
                        "derive attribute"
                    } else {
                        "attribute macro"
                    };
                    self.external(owner, &pkg, &path, kind, None, w.clone(), why);
                }
                Some(Cur::Items(v)) if kind == LinkKind::Inherits => {
                    if let Some(t) = v
                        .into_iter()
                        .find(|x| matches!(self.kind(*x), ItemKind::Trait | ItemKind::Macro))
                    {
                        let to = Target::Item(self.id(t).to_string());
                        self.link(owner, to, kind, None, w.clone(), "derive attribute", None);
                    }
                }
                _ => {}
            }
        }
    }

    fn on_use(&mut self, fi: usize, u: &ast::Use, owner: usize, at: u32) {
        if u.visibility().is_none() || self.kind(owner) != ItemKind::Mod {
            return;
        }
        let mut leaves = Vec::new();
        if let Some(tree) = u.use_tree() {
            flatten(&tree, &[], &mut leaves);
        }
        let module = self.module_of(owner);
        let krate = self.items[owner].krate;
        let w = self.witness(fi, at);
        for leaf in leaves.into_iter().filter(|l| l.alias.is_some()) {
            match self.resolve(&module, krate, &leaf.path, None, 1) {
                Some(Cur::Items(v)) => {
                    for t in v {
                        let to = Target::Item(self.id(t).to_string());
                        self.link(
                            owner,
                            to,
                            LinkKind::ReExports,
                            None,
                            w.clone(),
                            "`pub use`",
                            None,
                        );
                    }
                }
                Some(Cur::Ext { pkg, path }) if path.contains("::") => {
                    self.external(
                        owner,
                        &pkg,
                        &path,
                        LinkKind::ReExports,
                        None,
                        w.clone(),
                        "`pub use`",
                    );
                }
                _ => {}
            }
        }
    }

    fn web_framework(&self, krate: usize) -> Option<Framework> {
        let deps = &self.plan.packages[self.krates[krate].package].deps;
        if deps.iter().any(|d| d.package == "axum") {
            Some(Framework::Axum)
        } else if deps.iter().any(|d| d.package == "actix-web") {
            Some(Framework::Actix)
        } else {
            None
        }
    }

    /// Path prefix and middleware (outermost first) that apply to a route registered by `call`.
    fn route_ctx(
        &self,
        call: &ast::MethodCallExpr,
        fw: &Framework,
        depth: u8,
    ) -> (String, Vec<String>) {
        let top = chain_top(call);
        let (calls, base) = chain(&top);
        let i = calls
            .iter()
            .position(|c| c.syntax() == call.syntax())
            .unwrap_or(0);
        let named = |c: &ast::MethodCallExpr, names: &[&str]| {
            c.name_ref()
                .is_some_and(|n| names.contains(&n.text().to_string().as_str()))
        };
        let layer = |c: &ast::MethodCallExpr| {
            c.arg_list()
                .and_then(|a| a.args().next())
                .and_then(|a| middleware_name(&a))
        };
        let mut mids: Vec<String> = match fw {
            Framework::Axum => calls[i + 1..]
                .iter()
                .filter(|c| named(c, &["layer", "route_layer"]))
                .filter_map(layer)
                .collect(),
            _ => calls
                .iter()
                .filter(|c| named(c, &["wrap"]))
                .filter_map(layer)
                .collect(),
        };
        mids.reverse();
        let mut prefix = String::new();
        if let Some(ast::Expr::CallExpr(b)) = &base
            && b.expr()
                .as_ref()
                .and_then(path_of)
                .and_then(|p| segs(&p))
                .is_some_and(|s| s.last().is_some_and(|l| l == "scope"))
            && let Some(p) = b
                .arg_list()
                .and_then(|a| a.args().next())
                .and_then(|a| literal(&a))
        {
            prefix = p;
        }
        let outer = top
            .parent()
            .filter(|p| p.kind() == SyntaxKind::ARG_LIST)
            .and_then(|p| p.parent())
            .and_then(ast::MethodCallExpr::cast);
        if let Some(outer) = outer
            && depth < 4
        {
            let (mut p, mut m) = self.route_ctx(&outer, fw, depth + 1);
            if named(&outer, &["nest"])
                && let Some(lit) = outer
                    .arg_list()
                    .and_then(|a| a.args().next())
                    .and_then(|a| literal(&a))
            {
                p.push_str(&lit);
            }
            p.push_str(&prefix);
            m.extend(mids);
            return (p, m);
        }
        (prefix, mids)
    }

    fn on_route(&mut self, fi: usize, m: &ast::MethodCallExpr, owner: usize, at: u32) {
        let krate = self.items[owner].krate;
        let Some(fw) = self.web_framework(krate) else {
            return;
        };
        let args: Vec<ast::Expr> = m.arg_list().map(|a| a.args().collect()).unwrap_or_default();
        let [path_arg, router] = args.as_slice() else {
            return;
        };
        let Some(route) = literal(path_arg) else {
            return;
        };
        let mut found: Vec<(String, ast::Path)> = Vec::new();
        for n in router.syntax().descendants() {
            if let Some(c) = ast::CallExpr::cast(n.clone()) {
                let verb = c
                    .expr()
                    .as_ref()
                    .and_then(path_of)
                    .and_then(|p| segs(&p))
                    .and_then(|s| s.last().cloned());
                if let Some(v) = verb.filter(|v| VERBS.contains(&v.as_str()))
                    && let Some(h) = c
                        .arg_list()
                        .and_then(|a| a.args().next())
                        .as_ref()
                        .and_then(path_of)
                {
                    found.push((v, h));
                }
            } else if let Some(c) = ast::MethodCallExpr::cast(n) {
                let name = c
                    .name_ref()
                    .map(|n| n.text().to_string())
                    .unwrap_or_default();
                let Some(h) = c
                    .arg_list()
                    .and_then(|a| a.args().next())
                    .as_ref()
                    .and_then(path_of)
                else {
                    continue;
                };
                if VERBS.contains(&name.as_str()) {
                    found.push((name, h));
                } else if name == "to" {
                    let verb = c
                        .receiver()
                        .into_iter()
                        .flat_map(|r| r.syntax().descendants())
                        .find_map(|d| {
                            let s = segs(&ast::Path::cast(d)?)?;
                            s.last().filter(|l| VERBS.contains(&l.as_str())).cloned()
                        });
                    found.push((verb.unwrap_or_else(|| "any".into()), h));
                }
            }
        }
        let (prefix, mids) = self.route_ctx(m, &fw, 0);
        let module = self.module_of(owner);
        let w = self.witness(fi, at);
        for (verb, handler) in found {
            let Some(s) = segs(&handler) else { continue };
            let Some(Cur::Items(v)) =
                self.resolve(&module, krate, &s, self.self_of.get(&owner).copied(), 0)
            else {
                continue;
            };
            let Some(h) = v.into_iter().find(|x| self.kind(*x) == ItemKind::Fn) else {
                continue;
            };
            let name = format!("{} {prefix}{route}", verb.to_uppercase());
            let stack = if mids.is_empty() {
                String::new()
            } else {
                format!("; middleware, outermost first: {}", mids.join(" → "))
            };
            let why = format!("route `{name}`{stack}");
            self.link(
                owner,
                Target::Item(self.id(h).to_string()),
                LinkKind::Routes,
                None,
                w.clone(),
                &why,
                None,
            );
            let order = self.order(fi);
            self.routes.push(Route {
                order,
                owner,
                handler: h,
                name,
                framework: fw.clone(),
                witness: w.clone(),
            });
        }
    }

    /// actix's attribute routes: `#[get("/x")] async fn h()`.
    fn attribute_routes(&mut self) {
        let mut first: HashMap<usize, usize> = HashMap::new();
        for (i, it) in self.items.iter().enumerate() {
            first.entry(it.file).or_insert(i);
        }
        for i in 0..self.items.len() {
            if self.kind(i) != ItemKind::Fn
                || self.web_framework(self.items[i].krate) != Some(Framework::Actix)
            {
                continue;
            }
            for a in self.items[i].f.attrs.clone() {
                let Some((verb, rest)) = a.trim_start_matches("#[").split_once('(') else {
                    continue;
                };
                if !VERBS.contains(&verb) {
                    continue;
                }
                let Some(route) = unquote(rest) else { continue };
                let span = self.items[i].f.item.span;
                let witness = Witness::Span {
                    file: self.items[i].f.item.file.clone(),
                    line: span.line,
                    col: Some(span.col),
                };
                let name = format!("{} {route}", verb.to_uppercase());
                self.routes.push(Route {
                    order: [
                        1,
                        self.walk_of(self.items[i].file),
                        (i - first[&self.items[i].file]) as u32,
                    ],
                    owner: i,
                    handler: i,
                    name,
                    framework: Framework::Actix,
                    witness,
                });
            }
        }
    }

    fn finish(mut self) -> Output {
        self.attribute_routes();
        let mut out = Output::default();
        for f in &self.files {
            out.walk.push(f.path.clone());
            out.rust_files.insert(f.path.clone());
            out.files.entry(f.path.clone()).or_default();
        }
        for &fi in &self.unresolved {
            let f = &self.files[fi];
            let package = &self.plan.packages[self.krates[f.krate].package].name;
            out.unresolved
                .entry(package.clone())
                .or_default()
                .push(f.path.clone());
        }
        // Each file holds what it declares and the links it makes.
        for it in &self.items {
            let parts = out.files.entry(it.f.item.file.clone()).or_default();
            parts.items.push(it.f.item.clone());
            if it.f.is_test {
                parts.notes.tests.push(it.f.item.id.clone());
            }
            if !matches!(
                it.f.item.kind,
                ItemKind::Mod | ItemKind::Impl | ItemKind::Trait
            ) && items::has_unbounded_loop(&it.f.node)
            {
                parts.notes.loops.push(it.f.item.id.clone());
            }
        }
        for l in self.links.values() {
            if let Some(i) = self.by_id.get(&l.from) {
                out.files
                    .entry(self.items[*i].f.item.file.clone())
                    .or_default()
                    .links
                    .push(l.clone());
            }
        }

        // And notes what its bodies contribute to facts that span files (`assemble`).
        let file = |i: usize| self.items[i].f.item.file.clone();
        let mut notes: BTreeMap<String, Notes> = BTreeMap::new();
        for r in &self.routes {
            notes
                .entry(file(r.owner))
                .or_default()
                .routes
                .push(RouteNote {
                    order: r.order,
                    handler: self.id(r.handler).to_string(),
                    name: r.name.clone(),
                    framework: r.framework.clone(),
                    witness: r.witness.clone(),
                });
        }
        for s in &self.spawns {
            notes
                .entry(file(s.owner))
                .or_default()
                .spawns
                .push(SpawnNote {
                    order: s.order,
                    owner: self.id(s.owner).to_string(),
                    target: self.id(s.target).to_string(),
                    in_loop: s.in_loop,
                    witness: s.witness.clone(),
                });
        }
        let mut sql: BTreeMap<String, BTreeSet<SqlNote>> = BTreeMap::new();
        for u in &self.sql_uses {
            sql.entry(file(u.owner)).or_default().insert(SqlNote {
                owner: self.id(u.owner).to_string(),
                table: u.touch.table.clone(),
                inserts: u.touch.inserts,
                dequeues: u.touch.dequeues,
            });
        }
        for (f, uses) in sql {
            notes.entry(f).or_default().sql = uses.into_iter().collect();
        }
        for (f, touched) in std::mem::take(&mut self.touched) {
            notes.entry(f).or_default().touched = touched;
        }
        // Notes are taken in visit order: the first per file is the one that can win.
        for (fi, order, host, w) in &self.http {
            let n = notes.entry(self.files[*fi].path.clone()).or_default();
            if !n.hosts.iter().any(|h| &h.host == host) {
                n.hosts.push(HostNote {
                    order: *order,
                    host: host.clone(),
                    witness: w.clone(),
                });
            }
        }
        for (fi, order, w) in &self.tty {
            let n = notes.entry(self.files[*fi].path.clone()).or_default();
            n.tty.get_or_insert((*order, w.clone()));
        }
        for (f, n) in notes {
            let parts = out.files.entry(f).or_default();
            let tests = std::mem::take(&mut parts.notes.tests);
            let loops = std::mem::take(&mut parts.notes.loops);
            parts.notes = Notes { tests, loops, ..n };
        }
        out
    }
}

/// Whether a call expression is the direct argument of a spawn call.
fn is_spawn_arg(call: &SyntaxNode) -> bool {
    let outer = call
        .parent()
        .filter(|p| p.kind() == SyntaxKind::ARG_LIST)
        .and_then(|p| p.parent())
        .and_then(ast::CallExpr::cast);
    let callee = outer
        .and_then(|o| o.expr())
        .as_ref()
        .and_then(path_of)
        .and_then(|p| segs(&p));
    callee.is_some_and(|s| s.last().is_some_and(|l| SPAWNS.contains(&l.as_str())))
}

/// A host named by a URL literal in the text, when there is exactly one kind of it.
fn host_in(text: &str) -> Option<String> {
    let mut hosts = BTreeSet::new();
    for scheme in ["https://", "http://"] {
        for (i, _) in text.match_indices(scheme) {
            let rest = &text[i + scheme.len()..];
            let host: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '-'))
                .collect();
            if host.contains('.') {
                hosts.insert(host);
            }
        }
    }
    (hosts.len() == 1)
        .then(|| hosts.into_iter().next())
        .flatten()
}

fn flatten(tree: &ast::UseTree, prefix: &[String], out: &mut Vec<UseLeaf>) {
    let mut path = prefix.to_vec();
    if let Some(p) = tree.path() {
        match segs(&p) {
            Some(s) => path.extend(s),
            None => return,
        }
    }
    if let Some(list) = tree.use_tree_list() {
        for t in list.use_trees() {
            flatten(&t, &path, out);
        }
        return;
    }
    if tree.star_token().is_some() {
        out.push(UseLeaf { alias: None, path });
        return;
    }
    // `use a::b::{self}` names `b`.
    if path.last().is_some_and(|l| l == "self") && path.len() > 1 {
        path.pop();
    }
    let alias = match tree.rename() {
        Some(r) => match r.name() {
            Some(n) => n.text().to_string(),
            None => return,
        },
        None => match path.last() {
            Some(l) => l.clone(),
            None => return,
        },
    };
    out.push(UseLeaf {
        alias: Some(alias),
        path,
    });
}

/// Run the pass over the unit `plan` describes. `tree_files` are the repository's files, for
/// finding migrations. With a rust-analyzer `session`, links come from the type-checked view
/// and only the pattern links stay guessed.
pub fn run(
    root: &Path,
    plan: &UnitPlan,
    scope: &str,
    tree_files: &[PathBuf],
    session: Option<&crate::ra::Session>,
) -> Result<Output> {
    let mut unit = Unit::load(root, plan, scope).context("reading the unit's sources")?;
    for fi in 0..unit.files.len() {
        unit.visit_file(fi);
    }
    if let Some(session) = session {
        unit.resolve_links(session);
    }
    let mut out = unit.finish();
    // Tables from migrations.
    for path in tree_files {
        let p = path.to_string_lossy().replace('\\', "/");
        if !(p.ends_with(".sql") && p.split('/').any(|s| s == "migrations")) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        let tables: Vec<CreatedTable> = sql::created_tables(&text)
            .into_iter()
            .map(|c| CreatedTable {
                name: c.name,
                line: c.line,
            })
            .collect();
        if !tables.is_empty() {
            out.files.entry(p).or_default().notes.tables = tables;
        }
    }
    Ok(out)
}
