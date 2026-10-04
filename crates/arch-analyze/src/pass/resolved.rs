//! Links from the type-checked view. Every name in the unit's files is resolved by
//! rust-analyzer, the definition is mapped back to an item of the walker (or to an external
//! crate), and the same table as the syntax pass turns name-in-context into a link, now
//! `resolved`. Pattern links (routes, hand-offs, SQL, wiring, HTTP hosts) stay as guessed.

use std::collections::{HashMap, HashSet};

use arch_facts::{Access, ItemKind, LinkKind, Target};
use ra_ap_hir::{AsAssocItem, AssocItemContainer, Semantics, Variant, attach_db};
use ra_ap_ide::TryToNav;
use ra_ap_ide_db::RootDatabase;
use ra_ap_ide_db::base_db::CrateOrigin;
use ra_ap_ide_db::defs::{Definition, NameClass, NameRefClass};
use ra_ap_syntax::ast::{self, AstNode, HasVisibility};
use ra_ap_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use super::{Ctx, Tgt, Unit};
use crate::ra::Session;

/// Links the patterns produce: kept from the syntax pass when rust-analyzer provides the rest.
fn is_pattern(l: &arch_facts::Link) -> bool {
    matches!(
        l.kind,
        LinkKind::Routes
            | LinkKind::HandsOff
            | LinkKind::Queues
            | LinkKind::Wires
            | LinkKind::Inherits
            | LinkKind::Decorates
    ) || matches!(&l.to, Target::Table(_))
        || matches!(&l.to, Target::External(id) if !id.starts_with("external:crate:"))
}

struct Resolver<'a, 'db> {
    sema: Semantics<'db, RootDatabase>,
    session: &'a Session,
    /// (file index, offset of the item's name) → item.
    names: HashMap<(usize, u32), usize>,
    files: HashMap<String, usize>,
    cache: HashMap<Definition<'db>, Option<Tgt>>,
}

impl Unit<'_> {
    /// Replace the guessed links of the files rust-analyzer has a module for with resolved ones.
    /// `only` limits the resolution to some of the unit's files, by index.
    pub(super) fn resolve_links(&mut self, session: &Session, only: Option<&HashSet<usize>>) {
        attach_db(&session.db, || self.resolve_attached(session, only));
    }

    fn resolve_attached(&mut self, session: &Session, only: Option<&HashSet<usize>>) {
        let mut names = HashMap::new();
        for (i, it) in self.items.iter().enumerate() {
            let name = it.f.node.children().find(|c| c.kind() == SyntaxKind::NAME);
            if let Some(n) = name {
                names.insert((it.file, u32::from(n.text_range().start())), i);
            }
        }
        let files = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();
        let mut r = Resolver {
            sema: Semantics::new(&session.db),
            session,
            names,
            files,
            cache: HashMap::new(),
        };

        // A file is resolved when rust-analyzer has it in a crate's module tree: one it holds
        // outside every crate (a module a `cfg` compiles out) keeps its guessed links.
        let (loaded, unresolved): (Vec<_>, Vec<_>) = (0..self.files.len())
            .map(|fi| {
                let id = session
                    .file_id(&self.files[fi].path)
                    .filter(|id| r.sema.file_to_module_def(*id).is_some());
                (fi, id)
            })
            .partition(|(_, id)| id.is_some());
        self.unresolved = unresolved.into_iter().map(|(fi, _)| fi).collect();
        let loaded: Vec<_> = loaded
            .into_iter()
            .filter(|(fi, _)| only.is_none_or(|o| o.contains(fi)))
            .collect();
        let loaded: Vec<(usize, ra_ap_vfs::FileId)> = loaded
            .into_iter()
            .filter_map(|(fi, id)| Some((fi, id?)))
            .collect();
        let resolved_files: std::collections::HashSet<&str> = loaded
            .iter()
            .map(|(fi, _)| self.files[*fi].path.as_str())
            .collect();
        let by_id = &self.by_id;
        let items = &self.items;
        self.links.retain(|_, l| {
            let file = by_id.get(&l.from).map(|i| items[*i].f.item.file.as_str());
            is_pattern(l) || !file.is_some_and(|f| resolved_files.contains(f))
        });
        self.touched.clear();
        let kept: Vec<(String, String)> = self
            .links
            .values()
            .filter_map(|l| match &l.to {
                Target::External(id) => {
                    l.reason.as_ref()?;
                    let file = self.items[*self.by_id.get(&l.from)?].f.item.file.clone();
                    Some((file, id.strip_prefix("external:crate:")?.to_string()))
                }
                _ => None,
            })
            .collect();
        for (file, pkg) in kept {
            self.touched
                .entry(file)
                .or_default()
                .entry(pkg)
                .or_default();
        }

        self.resolved = true;
        for (fi, file_id) in loaded {
            let src = r.sema.parse_guess_edition(file_id);
            let tokens: Vec<SyntaxToken> = src
                .syntax()
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| {
                    matches!(
                        t.kind(),
                        SyntaxKind::IDENT | SyntaxKind::SELF_TYPE_KW | SyntaxKind::INT_NUMBER
                    )
                })
                .collect();
            for token in tokens {
                let at: u32 = token.text_range().start().into();
                let Some(owner) = self.owner_at(fi, at) else {
                    continue;
                };
                // In plain code the token classifies where it stands. Inside a macro call or an
                // attribute macro's input it does not, and its expansion is asked instead.
                if !self.on_token(&mut r, fi, &token, owner, at) {
                    for d in r.sema.descend_into_macros(token.clone()) {
                        if d != token {
                            self.on_token(&mut r, fi, &d, owner, at);
                        }
                    }
                }
            }
        }
        self.resolved = false;
    }

    /// Classify one token and emit its link. False when the token names nothing here.
    fn on_token<'db>(
        &mut self,
        r: &mut Resolver<'_, 'db>,
        fi: usize,
        token: &SyntaxToken,
        owner: usize,
        at: u32,
    ) -> bool {
        let Some(parent) = token.parent() else {
            return false;
        };
        if let Some(name) = ast::Name::cast(parent.clone()) {
            // `None` in a pattern is syntactically a new name and semantically a reference.
            if let Some(NameClass::ConstReference(def)) = NameClass::classify(&r.sema, &name)
                && let Some(tgt) = r.target(self, def)
            {
                let w = self.witness(fi, at);
                self.emit(owner, tgt, &Ctx::Pattern, name.syntax(), w);
                return true;
            }
            return false;
        }
        let Some(name_ref) = ast::NameRef::cast(parent) else {
            return false;
        };
        let def = match NameRefClass::classify(&r.sema, &name_ref) {
            Some(NameRefClass::Definition(def, _)) => def,
            Some(NameRefClass::FieldShorthand { field_ref, .. }) => Definition::Field(field_ref),
            _ => return false,
        };
        let Some(holder) = name_ref.syntax().parent() else {
            return true;
        };
        let w = self.witness(fi, at);
        match holder.kind() {
            SyntaxKind::METHOD_CALL_EXPR => {
                if let Some(tgt) = r.target(self, def) {
                    self.emit(owner, tgt, &Ctx::Call, &holder, w);
                }
            }
            SyntaxKind::FIELD_EXPR => self.on_field_def(r, def, &holder, owner, w),
            SyntaxKind::PATH_SEGMENT => {
                let Some(path) = holder.parent().and_then(ast::Path::cast) else {
                    return true;
                };
                if path
                    .syntax()
                    .parent()
                    .is_some_and(|p| p.kind() == SyntaxKind::PATH)
                {
                    // A qualifier (`postgres` in `postgres::connect`): the last segment speaks.
                    return true;
                }
                if let Some(tree) = path.syntax().parent().and_then(ast::UseTree::cast) {
                    self.on_use_def(r, def, &tree, owner, w);
                } else if let Some((ctx, expr)) = self.ctx_of(&path)
                    && let Some(tgt) = r.target(self, def)
                {
                    self.emit(owner, tgt, &ctx, &expr, w);
                }
            }
            _ => {}
        }
        true
    }

    fn on_field_def<'db>(
        &mut self,
        r: &mut Resolver<'_, 'db>,
        def: Definition<'db>,
        expr: &SyntaxNode,
        owner: usize,
        w: arch_facts::Witness,
    ) {
        let Definition::Field(field) = def else {
            return;
        };
        let db = r.sema.db;
        let adt = match field.parent_def(db) {
            Variant::Struct(s) => Definition::Adt(s.into()),
            Variant::Union(u) => Definition::Adt(u.into()),
            Variant::EnumVariant(v) => Definition::Adt(v.parent_enum(db).into()),
        };
        let Some(Tgt::Item(t)) = r.lookup(self, adt) else {
            return;
        };
        if self.self_of.get(&owner) == Some(&t) {
            return;
        }
        let writes = expr.parent().and_then(ast::BinExpr::cast).is_some_and(|b| {
            let assigns = b.op_token().is_some_and(|t| {
                let t = t.text();
                t.ends_with('=') && !matches!(t, "==" | "!=" | "<=" | ">=")
            });
            assigns && b.lhs().is_some_and(|l| l.syntax() == expr)
        });
        let access = if writes { Access::Write } else { Access::Read };
        let member = field.name(db).as_str().to_string();
        let to = Target::Item(self.id(t).to_string());
        self.link(
            owner,
            to,
            LinkKind::Reads,
            Some(member),
            w,
            "",
            Some(access),
        );
    }

    /// `pub use path::Name;` re-exports what the last segment names.
    fn on_use_def<'db>(
        &mut self,
        r: &mut Resolver<'_, 'db>,
        def: Definition<'db>,
        tree: &ast::UseTree,
        owner: usize,
        w: arch_facts::Witness,
    ) {
        let public = tree
            .syntax()
            .ancestors()
            .find_map(ast::Use::cast)
            .is_some_and(|u| u.visibility().is_some());
        if !public
            || tree.use_tree_list().is_some()
            || tree.star_token().is_some()
            || self.kind(owner) != ItemKind::Mod
        {
            return;
        }
        match r.target(self, def) {
            Some(Tgt::Item(t)) => {
                let to = Target::Item(self.id(t).to_string());
                self.link(owner, to, LinkKind::ReExports, None, w, "", None);
            }
            Some(Tgt::Ext { pkg, path }) if path.contains("::") => {
                self.external(owner, &pkg, &path, LinkKind::ReExports, None, w, "");
            }
            _ => {}
        }
    }
}

impl<'db> Resolver<'_, 'db> {
    /// What a definition is to the unit: one of its items, a member of one, or something in an
    /// external crate. `None` for std, locals, and definitions a macro generated.
    fn target(&mut self, unit: &Unit<'_>, def: Definition<'db>) -> Option<Tgt> {
        let db = self.sema.db;
        match def {
            Definition::EnumVariant(v) => {
                let name = v.name(db).as_str().to_string();
                match self.lookup(unit, Definition::Adt(v.parent_enum(db).into()))? {
                    Tgt::Item(e) => Some(Tgt::Variant(e, name)),
                    Tgt::Ext { pkg, path } => Some(Tgt::Ext {
                        pkg,
                        path: format!("{path}::{name}"),
                    }),
                    Tgt::Variant(..) => None,
                }
            }
            Definition::SelfType(imp) => {
                self.lookup(unit, Definition::Adt(imp.self_ty(db).as_adt()?))
            }
            Definition::Function(_)
            | Definition::Adt(_)
            | Definition::Trait(_)
            | Definition::Const(_)
            | Definition::Static(_)
            | Definition::TypeAlias(_)
            | Definition::Macro(_) => self.lookup(unit, def),
            _ => None,
        }
    }

    fn lookup(&mut self, unit: &Unit<'_>, def: Definition<'db>) -> Option<Tgt> {
        if let Some(t) = self.cache.get(&def) {
            return t.clone();
        }
        let t = self.lookup_uncached(unit, def);
        self.cache.insert(def, t.clone());
        t
    }

    fn lookup_uncached(&mut self, unit: &Unit<'_>, def: Definition<'db>) -> Option<Tgt> {
        let db = self.sema.db;
        let krate = def.krate(db)?;
        match krate.origin(db) {
            CrateOrigin::Lang(_) => None,
            CrateOrigin::Local { .. } => {
                let nav = def.try_to_nav(&self.sema)?.call_site;
                let file = self.session.rel_path(nav.file_id)?;
                let fi = *self.files.get(&file)?;
                let at: u32 = nav.focus_range?.start().into();
                self.names.get(&(fi, at)).copied().map(Tgt::Item)
            }
            CrateOrigin::Library { .. } | CrateOrigin::Rustc { .. } => {
                let crate_name = krate
                    .display_name(db)?
                    .canonical_name()
                    .as_str()
                    .to_string();
                let direct = unit
                    .plan
                    .packages
                    .iter()
                    .flat_map(|p| &p.deps)
                    .find(|d| d.ident == crate_name);
                let pkg = match (direct, unit.lock.get(&crate_name)) {
                    (Some(d), _) => d.package.clone(),
                    (None, Some((name, _))) => name.clone(),
                    (None, None) => crate_name.clone(),
                };
                let mut path = vec![crate_name];
                path.extend(
                    def.module(db)?
                        .path_to_root(db)
                        .into_iter()
                        .rev()
                        .filter_map(|m| m.name(db))
                        .map(|n| n.as_str().to_string()),
                );
                if let Definition::Function(f) = def
                    && let Some(assoc) = f.as_assoc_item(db)
                {
                    let container = match assoc.container(db) {
                        AssocItemContainer::Trait(t) => Some(t.name(db).as_str().to_string()),
                        AssocItemContainer::Impl(i) => i
                            .self_ty(db)
                            .as_adt()
                            .map(|a| a.name(db).as_str().to_string()),
                    };
                    path.extend(container);
                }
                path.push(def.name(db)?.as_str().to_string());
                Some(Tgt::Ext {
                    pkg,
                    path: path.join("::"),
                })
            }
        }
    }
}
