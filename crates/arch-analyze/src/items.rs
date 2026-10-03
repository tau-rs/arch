//! The item walker: what one source file declares, read from its syntax tree alone.
//!
//! An item is what rust-analyzer calls an item, including associated items; enum variants and
//! struct fields are not (arch issue #14). Ids never contain a line number (MAP-1).

use std::collections::HashMap;

use arch_facts::{Item, ItemFlags, ItemKind, Span, Visibility};
use ra_ap_syntax::ast::{self, AstNode, HasAttrs, HasModuleItem, HasName, HasVisibility};
use ra_ap_syntax::{SyntaxKind, SyntaxNode, TextRange};

/// Byte offset → 1-based line and column (in characters).
#[derive(Debug, Clone)]
pub struct Lines {
    text: String,
    starts: Vec<u32>,
}

impl Lines {
    /// Index a file's text.
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i as u32 + 1),
        );
        Lines {
            text: text.to_string(),
            starts,
        }
    }

    /// Line and column of a byte offset.
    pub fn pos(&self, offset: u32) -> (u32, u32) {
        let offset = offset.min(self.text.len() as u32);
        let line = self.starts.partition_point(|s| *s <= offset) - 1;
        let start = self.starts[line] as usize;
        let col = self
            .text
            .get(start..offset as usize)
            .map_or(0, |s| s.chars().count());
        (line as u32 + 1, col as u32 + 1)
    }

    /// The span of a range; `end` is inclusive.
    pub fn span(&self, range: TextRange) -> Span {
        let (line, col) = self.pos(range.start().into());
        let end: u32 = range.end().into();
        let (end_line, end_col) = self.pos(end.saturating_sub(1).max(range.start().into()));
        Span {
            line,
            col,
            end_line,
            end_col,
        }
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The crate a file is walked for.
#[derive(Debug, Clone)]
pub struct CrateCtx {
    /// Id prefix: the lib's crate name, or `<name>[bin:<target>]` for another target.
    pub label: String,
    /// Cargo package name, the item's `crate`.
    pub package: String,
    /// Unit scope id.
    pub scope: String,
    /// The crate root is a bin: its `main` is an entry.
    pub is_bin: bool,
}

/// One item found, with what the link pass needs to read it again.
#[derive(Debug, Clone)]
pub struct Found {
    /// The fact.
    pub item: Item,
    /// Its node.
    pub node: SyntaxNode,
    /// A `#[test]` function (any attribute path ending in `test`).
    pub is_test: bool,
    /// Module path of whatever sits inside this item: an inline module's own path, else the
    /// path of the module the item is in.
    pub inner_module: Vec<String>,
    /// For an `impl`: its self type as written.
    pub self_ty: Option<String>,
    /// For an `impl`: the trait as written.
    pub trait_: Option<String>,
    /// Attributes, whitespace removed (`#[get("/x")]`).
    pub attrs: Vec<String>,
}

/// An out-of-line `mod x;`: a file to walk next.
#[derive(Debug, Clone)]
pub struct ModDecl {
    /// Module name.
    pub name: String,
    /// Path of the module that declares it.
    pub parent: Vec<String>,
    /// `#[path = "…"]`, when given.
    pub path_attr: Option<String>,
}

/// Disambiguates repeated ids inside one crate: the second gets `.2`.
#[derive(Debug, Default)]
pub struct Ids(HashMap<String, u32>);

impl Ids {
    fn unique(&mut self, id: String) -> String {
        let n = self.0.entry(id.clone()).or_insert(0);
        *n += 1;
        if *n == 1 { id } else { format!("{id}.{n}") }
    }
}

/// The schema's spelling of an item kind, as used in ids.
pub fn kind_str(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Fn => "fn",
        ItemKind::Struct => "struct",
        ItemKind::Enum => "enum",
        ItemKind::Trait => "trait",
        ItemKind::Impl => "impl",
        ItemKind::Mod => "mod",
        ItemKind::Macro => "macro",
        ItemKind::Const => "const",
        ItemKind::Static => "static",
        ItemKind::TypeAlias => "type-alias",
        ItemKind::Union => "union",
    }
}

/// Collapse whitespace so a type written over several lines yields one id.
pub fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn attrs_of(node: &impl HasAttrs) -> Vec<String> {
    node.attrs()
        .map(|a| {
            a.syntax()
                .text()
                .to_string()
                .split_whitespace()
                .collect::<String>()
        })
        .collect()
}

fn cfg_of(attrs: &[String]) -> Option<String> {
    attrs.iter().find_map(|a| {
        a.strip_prefix("#[cfg(")?
            .strip_suffix(")]")
            .map(str::to_string)
    })
}

fn is_test_attr(attrs: &[String]) -> bool {
    attrs.iter().any(|a| {
        let path = a
            .trim_start_matches("#[")
            .split(['(', ']'])
            .next()
            .unwrap_or("");
        path == "test" || path.ends_with("::test")
    })
}

fn visibility(node: &impl HasVisibility) -> Visibility {
    match node.visibility().map(|v| {
        v.syntax()
            .text()
            .to_string()
            .split_whitespace()
            .collect::<String>()
    }) {
        None => Visibility::Private,
        Some(v) if v == "pub" => Visibility::Pub,
        Some(v) if v == "pub(crate)" => Visibility::Crate,
        Some(v) if v == "pub(super)" => Visibility::Super,
        Some(v) if v == "pub(self)" => Visibility::Private,
        Some(_) => Visibility::Restricted,
    }
}

fn has_unsafe_block(node: &SyntaxNode) -> bool {
    node.descendants()
        .filter_map(ast::BlockExpr::cast)
        .any(|b| b.unsafe_token().is_some())
}

struct Walker<'a> {
    krate: &'a CrateCtx,
    file: &'a str,
    lines: &'a Lines,
    ids: &'a mut Ids,
    found: Vec<Found>,
    decls: Vec<ModDecl>,
}

struct Scope<'a> {
    module: &'a [String],
    /// Id of the enclosing `impl` or `trait`, for associated items.
    parent: Option<&'a str>,
    cfg: Option<&'a str>,
    is_root: bool,
}

impl Walker<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: ItemKind,
        name: &str,
        node: &SyntaxNode,
        vis: Visibility,
        scope: &Scope<'_>,
        attrs: Vec<String>,
        mut flags: ItemFlags,
    ) -> usize {
        let prefix = match scope.parent {
            Some(p) => p.split('#').next().unwrap_or(p).to_string(),
            None => std::iter::once(self.krate.label.as_str())
                .chain(scope.module.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("::"),
        };
        let id = self
            .ids
            .unique(format!("{prefix}::{name}#{}", kind_str(kind)));
        flags.cfg = cfg_of(&attrs).or_else(|| scope.cfg.map(str::to_string));
        let is_test = kind == ItemKind::Fn && is_test_attr(&attrs);
        // Entries are `main`, framework-held handlers and spawned workers (ADR 0028). A test is
        // not one: what it exercises is carried by its `tests` links.
        flags.entry = kind == ItemKind::Fn
            && name == "main"
            && self.krate.is_bin
            && scope.is_root
            && scope.parent.is_none();
        self.found.push(Found {
            item: Item {
                id,
                kind,
                name: name.to_string(),
                file: self.file.to_string(),
                span: self.lines.span(node.text_range()),
                crate_name: self.krate.package.clone(),
                module: scope.module.join("::"),
                visibility: vis,
                reexported: false,
                flags,
                scope: self.krate.scope.clone(),
                parent: scope.parent.map(str::to_string),
            },
            node: node.clone(),
            is_test,
            inner_module: scope.module.to_vec(),
            self_ty: None,
            trait_: None,
            attrs,
        });
        self.found.len() - 1
    }

    fn items(&mut self, items: impl Iterator<Item = ast::Item>, scope: &Scope<'_>) {
        for item in items {
            let attrs = attrs_of(&item);
            let node = item.syntax().clone();
            let name = |n: Option<ast::Name>| n.map(|n| n.text().to_string());
            match &item {
                ast::Item::Fn(f) => {
                    let Some(n) = name(f.name()) else { continue };
                    let flags = ItemFlags {
                        r#unsafe: f.unsafe_token().is_some() || has_unsafe_block(&node),
                        ..Default::default()
                    };
                    self.push(ItemKind::Fn, &n, &node, visibility(f), scope, attrs, flags);
                }
                ast::Item::Struct(s) => {
                    let Some(n) = name(s.name()) else { continue };
                    self.push(
                        ItemKind::Struct,
                        &n,
                        &node,
                        visibility(s),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::Enum(e) => {
                    let Some(n) = name(e.name()) else { continue };
                    self.push(
                        ItemKind::Enum,
                        &n,
                        &node,
                        visibility(e),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::Union(u) => {
                    let Some(n) = name(u.name()) else { continue };
                    self.push(
                        ItemKind::Union,
                        &n,
                        &node,
                        visibility(u),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::Const(c) => {
                    let Some(n) = name(c.name()) else { continue };
                    self.push(
                        ItemKind::Const,
                        &n,
                        &node,
                        visibility(c),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::Static(s) => {
                    let Some(n) = name(s.name()) else { continue };
                    self.push(
                        ItemKind::Static,
                        &n,
                        &node,
                        visibility(s),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::TypeAlias(t) => {
                    let Some(n) = name(t.name()) else { continue };
                    self.push(
                        ItemKind::TypeAlias,
                        &n,
                        &node,
                        visibility(t),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::MacroRules(m) => {
                    let Some(n) = name(m.name()) else { continue };
                    self.push(
                        ItemKind::Macro,
                        &n,
                        &node,
                        visibility(m),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::MacroDef(m) => {
                    let Some(n) = name(m.name()) else { continue };
                    self.push(
                        ItemKind::Macro,
                        &n,
                        &node,
                        visibility(m),
                        scope,
                        attrs,
                        ItemFlags::default(),
                    );
                }
                ast::Item::Trait(t) => {
                    let Some(n) = name(t.name()) else { continue };
                    let flags = ItemFlags {
                        r#unsafe: t.unsafe_token().is_some(),
                        ..Default::default()
                    };
                    let i = self.push(
                        ItemKind::Trait,
                        &n,
                        &node,
                        visibility(t),
                        scope,
                        attrs,
                        flags,
                    );
                    if let Some(list) = t.assoc_item_list() {
                        self.assoc(list, i, scope);
                    }
                }
                ast::Item::Impl(imp) => {
                    let Some(self_ty) = imp
                        .self_ty()
                        .map(|t| squash(&t.syntax().text().to_string()))
                    else {
                        continue;
                    };
                    let trait_ = imp.trait_().map(|t| squash(&t.syntax().text().to_string()));
                    let n = match &trait_ {
                        Some(t) => format!("impl {t} for {self_ty}"),
                        None => format!("impl {self_ty}"),
                    };
                    let flags = ItemFlags {
                        r#unsafe: imp.unsafe_token().is_some(),
                        drop: trait_.as_deref() == Some("Drop"),
                        ..Default::default()
                    };
                    let i = self.push(
                        ItemKind::Impl,
                        &n,
                        &node,
                        Visibility::Private,
                        scope,
                        attrs,
                        flags,
                    );
                    self.found[i].self_ty = Some(self_ty);
                    self.found[i].trait_ = trait_;
                    if let Some(list) = imp.assoc_item_list() {
                        self.assoc(list, i, scope);
                    }
                }
                ast::Item::Module(m) => {
                    let Some(n) = name(m.name()) else { continue };
                    let i = self.push(
                        ItemKind::Mod,
                        &n,
                        &node,
                        visibility(m),
                        scope,
                        attrs.clone(),
                        ItemFlags::default(),
                    );
                    let mut inner = scope.module.to_vec();
                    inner.push(n.clone());
                    self.found[i].inner_module = inner.clone();
                    match m.item_list() {
                        Some(list) => {
                            let cfg = self.found[i].item.flags.cfg.clone();
                            let sub = Scope {
                                module: &inner,
                                parent: None,
                                cfg: cfg.as_deref(),
                                is_root: false,
                            };
                            self.items(list.items(), &sub);
                        }
                        None => {
                            let path_attr = attrs.iter().find_map(|a| {
                                a.strip_prefix("#[path=\"")?
                                    .strip_suffix("\"]")
                                    .map(str::to_string)
                            });
                            self.decls.push(ModDecl {
                                name: n,
                                parent: scope.module.to_vec(),
                                path_attr,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn assoc(&mut self, list: ast::AssocItemList, parent: usize, scope: &Scope<'_>) {
        let parent_id = self.found[parent].item.id.clone();
        let cfg = self.found[parent].item.flags.cfg.clone();
        let sub = Scope {
            module: scope.module,
            parent: Some(&parent_id),
            cfg: cfg.as_deref(),
            is_root: false,
        };
        let items = list.assoc_items().filter_map(|a| match a {
            ast::AssocItem::Fn(f) => Some(ast::Item::Fn(f)),
            ast::AssocItem::Const(c) => Some(ast::Item::Const(c)),
            ast::AssocItem::TypeAlias(t) => Some(ast::Item::TypeAlias(t)),
            ast::AssocItem::MacroCall(_) => None,
        });
        self.items(items, &sub);
    }
}

/// Walk one file. `module` is the path of the module the file defines; `is_root` adds the crate
/// root module item (`<label>#mod`), which re-exports and crate-level links hang from.
pub fn walk(
    src: &ast::SourceFile,
    krate: &CrateCtx,
    module: &[String],
    file: &str,
    lines: &Lines,
    is_root: bool,
    ids: &mut Ids,
) -> (Vec<Found>, Vec<ModDecl>) {
    let mut w = Walker {
        krate,
        file,
        lines,
        ids,
        found: Vec::new(),
        decls: Vec::new(),
    };
    if is_root {
        let id = w.ids.unique(format!("{}#mod", krate.label));
        w.found.push(Found {
            item: Item {
                id,
                kind: ItemKind::Mod,
                name: krate
                    .label
                    .split('[')
                    .next()
                    .unwrap_or(&krate.label)
                    .to_string(),
                file: file.to_string(),
                span: lines.span(src.syntax().text_range()),
                crate_name: krate.package.clone(),
                module: String::new(),
                visibility: Visibility::Pub,
                reexported: false,
                flags: ItemFlags::default(),
                scope: krate.scope.clone(),
                parent: None,
            },
            node: src.syntax().clone(),
            is_test: false,
            inner_module: Vec::new(),
            self_ty: None,
            trait_: None,
            attrs: Vec::new(),
        });
    }
    let scope = Scope {
        module,
        parent: None,
        cfg: None,
        is_root,
    };
    w.items(src.items(), &scope);
    (w.found, w.decls)
}

/// Whether a function body loops without a bound of its own: `loop` or `while` (ADR 0028 §4).
pub fn has_unbounded_loop(node: &SyntaxNode) -> bool {
    node.descendants()
        .any(|n| matches!(n.kind(), SyntaxKind::LOOP_EXPR | SyntaxKind::WHILE_EXPR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra_ap_syntax::Edition;

    fn walk_text(text: &str, is_bin: bool) -> (Vec<Found>, Vec<ModDecl>) {
        let src = ast::SourceFile::parse(text, Edition::CURRENT).tree();
        let krate = CrateCtx {
            label: "c".into(),
            package: "c".into(),
            scope: "unit:c".into(),
            is_bin,
        };
        walk(
            &src,
            &krate,
            &[],
            "src/lib.rs",
            &Lines::new(text),
            true,
            &mut Ids::default(),
        )
    }

    #[test]
    fn ids_follow_the_grammar_and_never_carry_a_line() {
        let (found, decls) = walk_text(
            "pub mod a;\nmod inline { pub(crate) fn f() {} }\npub struct S;\nimpl S { pub fn new() -> Self { S } const K: u8 = 1; }\n\
             impl  Default for S { fn default() -> Self { S } }\nimpl S { fn other(&self) {} }\n\
             pub trait T { fn m(&self); type Out; }\nmacro_rules! mac { () => {} }\ntype Alias = S;\nstatic ST: u8 = 0;\n",
            false,
        );
        let ids: Vec<&str> = found.iter().map(|f| f.item.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "c#mod",
                "c::a#mod",
                "c::inline#mod",
                "c::inline::f#fn",
                "c::S#struct",
                "c::impl S#impl",
                "c::impl S::new#fn",
                "c::impl S::K#const",
                "c::impl Default for S#impl",
                "c::impl Default for S::default#fn",
                "c::impl S#impl.2",
                "c::impl S::other#fn",
                "c::T#trait",
                "c::T::m#fn",
                "c::T::Out#type-alias",
                "c::mac#macro",
                "c::Alias#type-alias",
                "c::ST#static",
            ]
        );
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "a");
        let new = &found[6].item;
        assert_eq!(new.parent.as_deref(), Some("c::impl S#impl"));
        assert_eq!(new.visibility, Visibility::Pub);
        assert_eq!(found[3].item.module, "inline");
        assert_eq!(found[3].item.visibility, Visibility::Crate);
        assert_eq!(
            found[4].item.span,
            Span {
                line: 3,
                col: 1,
                end_line: 3,
                end_col: 13
            }
        );
    }

    #[test]
    fn flags_mark_entries_tests_cfg_unsafe_and_drop() {
        let (found, _) = walk_text(
            "fn main() {}\n#[cfg(test)]\nmod tests { #[test] fn t() {} #[tokio::test] async fn u() {} fn helper() {} }\n\
             unsafe fn raw() {}\nfn wraps() { unsafe { raw() } }\nstruct G;\nimpl Drop for G { fn drop(&mut self) {} }\n",
            true,
        );
        let by = |id: &str| {
            found
                .iter()
                .find(|f| f.item.id == id)
                .unwrap_or_else(|| panic!("{id}"))
        };
        assert!(by("c::main#fn").item.flags.entry);
        assert!(by("c::tests::t#fn").is_test && !by("c::tests::t#fn").item.flags.entry);
        assert!(by("c::tests::u#fn").is_test);
        assert!(!by("c::tests::helper#fn").item.flags.entry);
        assert_eq!(
            by("c::tests::helper#fn").item.flags.cfg.as_deref(),
            Some("test")
        );
        assert!(by("c::raw#fn").item.flags.r#unsafe && by("c::wraps#fn").item.flags.r#unsafe);
        assert!(by("c::impl Drop for G#impl").item.flags.drop);
        // In a lib, `main` is just a function.
        let (lib, _) = walk_text("fn main() {}", false);
        assert!(!lib[1].item.flags.entry);
    }

    #[test]
    fn positions_count_characters_not_bytes() {
        let l = Lines::new("// é\nfn f() {}\n");
        assert_eq!(l.pos(0), (1, 1));
        assert_eq!(l.pos(6), (2, 1));
        assert_eq!(Lines::new("é x").pos(3), (1, 3));
    }
}
