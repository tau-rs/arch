//! What `arch init` writes into `areas.toml`: areas, sides and order computed from the facts.
//!
//! - **Areas** (ADR 0029, decision 4): one per top-level module of the unit's root crate, a
//!   single file included. The crate root (`main.rs`, `lib.rs`) has none.
//! - **Sides** (ADR 0027, hexagon units only), first match wins: the module holds an entry →
//!   driving; it touches an I/O external, or implements a trait of the unit that an
//!   I/O-touching module also implements → driven; otherwise domain. Non-test code only.
//!   Entries are whatever the fact model lists (ADR 0028) plus both ends of a `routes` link: the
//!   handler, and the module that registers it (arch-design#29).
//! - **Split** (ADR 0027, decision 4): when a module's direct children give at least one driving
//!   and one driven, each child is an area of its own, named by its bare name, prefixed with the
//!   parent on a clash.
//! - **Order** (ADR 0029): within a side, the 1-based rank of the area's name, compared byte by
//!   byte.
//! - **Rule**: hexagon when the unit has an entry, layers otherwise (MAP-26). A layers unit gets
//!   order only.

use std::collections::{BTreeMap, BTreeSet};

use arch_facts::{
    AreaOverride, Areas, Column, ColumnRule, External, Facts, Item, ItemKind, LinkKind, Port,
    PortKind, Side, Target,
};

/// Compute the content of `areas.toml` for a unit that has none.
pub fn propose_areas(facts: &Facts) -> Areas {
    let root = source_root(facts);
    let items: BTreeMap<&str, &Item> = facts.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let groups = Groups::collect(facts, &items, &root);

    let hexagon = facts
        .entries
        .iter()
        .any(|e| items.get(e.item.as_str()).is_some_and(|i| !is_test_code(i)));

    // Areas: a top-level module, or its direct children when they mix driving and driven.
    let mut areas: Vec<(String, Option<Column>, Vec<String>)> = Vec::new();
    let mut taken: BTreeSet<String> = groups.top.keys().cloned().collect();
    for (name, group) in &groups.top {
        let children = groups.children.get(name);
        let split = hexagon
            && children.is_some_and(|c| {
                let sides: BTreeSet<Column> = c.values().map(|g| groups.side(g)).collect();
                sides.contains(&Column::Driving) && sides.contains(&Column::Driven)
            });
        if split {
            taken.remove(name);
            for (child, g) in children.into_iter().flatten() {
                let area = if taken.insert(child.clone()) {
                    child.clone()
                } else {
                    format!("{name}::{child}")
                };
                areas.push((area, Some(groups.side(g)), g.paths(&root, &[name, child])));
            }
        } else {
            let side = hexagon.then(|| groups.side(group));
            areas.push((name.clone(), side, group.paths(&root, &[name])));
        }
    }

    // Order: byte-wise rank of the name within its side (one sequence in a layers unit).
    areas.sort_by(|a, b| (a.1, a.0.as_bytes()).cmp(&(b.1, b.0.as_bytes())));
    let mut rank: BTreeMap<Option<Column>, u32> = BTreeMap::new();
    let areas = areas
        .into_iter()
        .map(|(name, side, paths)| {
            let order = rank.entry(side).or_insert(0);
            *order += 1;
            AreaOverride {
                name,
                paths,
                side,
                order: Some(*order),
            }
        })
        .collect();

    Areas {
        rule: Some(if hexagon {
            ColumnRule::Hexagon
        } else {
            ColumnRule::Layers
        }),
        main_bin: facts
            .repo
            .unit
            .main_target
            .strip_prefix("bin:")
            .map(str::to_string),
        areas,
    }
}

/// The facts the sides rule reads, for one module or one child module.
#[derive(Debug, Default)]
struct Group {
    files: BTreeSet<String>,
    holds_entry: bool,
    touches_io: bool,
    implements: BTreeSet<String>,
}

impl Group {
    /// `src/x/**` for a directory module, `src/x.rs` for a single file, both when the module has
    /// a file and a directory (ADR 0029, decision 4: never more than one module per area).
    fn paths(&self, root: &str, module: &[&String]) -> Vec<String> {
        let base = format!(
            "{root}{}",
            module
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("/")
        );
        let (file, dir) = (format!("{base}.rs"), format!("{base}/"));
        let mut paths = Vec::new();
        if self.files.contains(&file) {
            paths.push(file);
        }
        if self.files.iter().any(|f| f.starts_with(&dir)) {
            paths.push(format!("{dir}**"));
        }
        paths
    }
}

struct Groups {
    /// Top-level modules of the root crate.
    top: BTreeMap<String, Group>,
    /// Their direct children.
    children: BTreeMap<String, BTreeMap<String, Group>>,
    /// Unit traits implemented by a module that touches an I/O external.
    io_traits: BTreeSet<String>,
}

impl Groups {
    fn collect(facts: &Facts, items: &BTreeMap<&str, &Item>, root: &str) -> Self {
        let externals: BTreeMap<&str, &External> =
            facts.externals.iter().map(|e| (e.id.as_str(), e)).collect();
        let ports: BTreeMap<&str, &Port> = facts.ports.iter().map(|p| (p.id.as_str(), p)).collect();
        let mut top: BTreeMap<String, Group> = BTreeMap::new();
        let mut children: BTreeMap<String, BTreeMap<String, Group>> = BTreeMap::new();

        // Apply `f` to the group of the item's top-level module and to its child module's.
        let mut each = |item: &Item, f: &mut dyn FnMut(&mut Group)| {
            let Some((module, child)) = placed(item, root) else {
                return;
            };
            f(top.entry(module.to_string()).or_default());
            if let Some(child) = child {
                let of_module = children.entry(module.to_string()).or_default();
                f(of_module.entry(child.to_string()).or_default());
            }
        };

        for item in &facts.items {
            each(item, &mut |g| {
                g.files.insert(item.file.clone());
            });
        }
        let entry_items = facts.entries.iter().map(|e| e.item.as_str());
        // both ends of a `routes` link: the handler, and the module that mounts it
        // (arch-design#29)
        let routes = facts.links.iter().filter_map(|l| match (&l.kind, &l.to) {
            (LinkKind::Routes, Target::Item(id)) => Some([l.from.as_str(), id.as_str()]),
            _ => None,
        });
        for id in entry_items.chain(routes.flatten()) {
            if let Some(item) = items.get(id) {
                each(item, &mut |g| g.holds_entry = true);
            }
        }
        for link in &facts.links {
            let Some(from) = items.get(link.from.as_str()) else {
                continue;
            };
            let io = match &link.to {
                Target::Table(_) => true,
                Target::External(id) => externals.get(id.as_str()).is_some_and(|e| is_io(e.kind)),
                Target::Port(id) => ports
                    .get(id.as_str())
                    .is_some_and(|p| p.side == Side::Driven && is_io(p.kind)),
                Target::Item(_) => false,
            };
            if io {
                each(from, &mut |g| g.touches_io = true);
            }
            if let (LinkKind::Implements, Target::Item(id)) = (&link.kind, &link.to)
                && items
                    .get(id.as_str())
                    .is_some_and(|t| t.kind == ItemKind::Trait)
            {
                each(from, &mut |g| {
                    g.implements.insert(id.clone());
                });
            }
        }

        // "A module touching an I/O external": the finest modules we look at, so that a parent
        // which merely contains an adapter does not lend its traits to every sibling.
        let mut io_traits = BTreeSet::new();
        for (name, group) in &top {
            match children.get(name) {
                Some(kids) if !kids.is_empty() => {
                    for kid in kids.values().filter(|k| k.touches_io) {
                        io_traits.extend(kid.implements.iter().cloned());
                    }
                }
                _ if group.touches_io => io_traits.extend(group.implements.iter().cloned()),
                _ => {}
            }
        }
        Groups {
            top,
            children,
            io_traits,
        }
    }

    /// ADR 0027, decision 2: first match wins.
    fn side(&self, group: &Group) -> Column {
        if group.holds_entry {
            Column::Driving
        } else if group.touches_io || !group.implements.is_disjoint(&self.io_traits) {
            Column::Driven
        } else {
            Column::Domain
        }
    }
}

/// The top-level module an item belongs to, and its direct child module when it is deeper.
/// `None` for test code, the crate root, other crates, and modules written inline in the root file.
fn placed<'a>(item: &'a Item, root: &str) -> Option<(&'a str, Option<&'a str>)> {
    if is_test_code(item) || item.module.is_empty() {
        return None;
    }
    let rest = item.file.strip_prefix(root)?;
    let mut segments = item.module.split("::");
    let module = segments.next()?;
    if !(rest == format!("{module}.rs") || rest.starts_with(&format!("{module}/"))) {
        return None;
    }
    let child = segments.next().filter(|child| {
        let under = &rest[module.len()..];
        under == format!("/{child}.rs") || under.starts_with(&format!("/{child}/"))
    });
    Some((module, child))
}

/// The directory the root crate's modules live in, with a trailing slash: where `main.rs` or
/// `lib.rs` of the unit's crate is. `src/` when the facts hold no root item.
fn source_root(facts: &Facts) -> String {
    let root_file = |name: &str| {
        facts
            .items
            .iter()
            .filter(|i| i.module.is_empty())
            .find_map(|i| i.file.strip_suffix(name))
    };
    root_file("lib.rs")
        .or_else(|| root_file("main.rs"))
        .unwrap_or("src/")
        .to_string()
}

/// I/O kinds (ADR 0027): everything but libraries and the crate's own public surface.
pub(crate) fn is_io(kind: PortKind) -> bool {
    !matches!(kind, PortKind::Crate | PortKind::Pub | PortKind::Declared)
}

/// Whether an item is under a `test` cfg (ADR 0027: the rules read non-test code only).
pub(crate) fn is_test_code(item: &Item) -> bool {
    item.flags.cfg.as_deref().is_some_and(|cfg| {
        cfg.split(|c: char| !c.is_alphanumeric())
            .any(|w| w == "test")
    })
}
