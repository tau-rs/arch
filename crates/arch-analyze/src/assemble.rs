//! Facts that span files, derived when a tree is assembled (ADR 0002).
//!
//! A file's delta holds what the file declares and the links it makes. Some facts need more
//! than one file's body: a handler is an entry because a route in another file names it, an
//! inserter `queues` because another function dequeues with `SKIP LOCKED`, a crate external
//! lists what every file touches. A delta carrying them would go stale when that other file
//! changes. So each file leaves [`Notes`] on what its bodies contribute, and [`derive`] reads
//! the notes of every file of the tree into [`Assembled`] facts.
//!
//! Where several files offer the same fact (one entry per item, one port per name, one witness
//! per host), the first in the pass's visit order wins, as it did when the pass chose: each
//! note carries its [`Order`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use arch_facts::{
    Access, Assembled, Confidence, Entry, EntryKind, External, FileFacts, Framework, Item,
    LinkFlags, LinkKind, Port, PortKind, QueueUse, Section, Side, Table, Target, Witness,
};
use serde::{Deserialize, Serialize};

use crate::cargo::UnitPlan;

/// Where a note was taken in the pass: `[phase, file, sequence]`. Phase 0 is the walk over each
/// file's syntax, phase 1 what is read off items afterwards (actix's attribute routes). Stable
/// while the unit's declarations are: files and items are numbered by the module tree.
pub type Order = [u32; 3];

/// What one file's bodies contribute to facts that span files. Stored with the file's delta,
/// opaque to the store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Notes {
    /// Routes registered in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RouteNote>,
    /// Work handed to another task in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawns: Vec<SpawnNote>,
    /// Tables the file's SQL touches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sql: Vec<SqlNote>,
    /// External crates' paths the file touches, by package.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub touched: BTreeMap<String, BTreeSet<String>>,
    /// HTTP hosts the file calls, the first call per host.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<HostNote>,
    /// The file's first direct write to the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<(Order, Witness)>,
    /// The file's `#[test]` functions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// The file's items whose body loops without end.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loops: Vec<String>,
    /// Tables a migration creates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<CreatedTable>,
}

impl Notes {
    /// True when the file contributes nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A route: `handler` is held by the framework under `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteNote {
    /// Where it was found.
    pub order: Order,
    /// The handler's id.
    pub handler: String,
    /// `GET /x`.
    pub name: String,
    /// The framework.
    pub framework: Framework,
    /// Where the route is registered.
    pub witness: Witness,
}

/// `spawn(target(..))` in `owner`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnNote {
    /// Where it was found.
    pub order: Order,
    /// The spawning function's id.
    pub owner: String,
    /// The spawned function's id.
    pub target: String,
    /// The spawn sits in a loop.
    pub in_loop: bool,
    /// The spawn call.
    pub witness: Witness,
}

/// `owner`'s SQL touches `table`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SqlNote {
    /// The function's id.
    pub owner: String,
    /// The table.
    pub table: String,
    /// It inserts.
    pub inserts: bool,
    /// It claims rows with `FOR UPDATE SKIP LOCKED`.
    pub dequeues: bool,
}

/// A call to an HTTP host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostNote {
    /// Where it was found.
    pub order: Order,
    /// The host.
    pub host: String,
    /// The call.
    pub witness: Witness,
}

/// `CREATE TABLE name` at `line` of a migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedTable {
    /// The table.
    pub name: String,
    /// Its line.
    pub line: u32,
}

/// Read a delta's notes back.
pub fn notes_of(delta: &FileFacts) -> Notes {
    delta
        .notes
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Notes as stored with a delta: `None` when there are none.
pub fn to_value(notes: &Notes) -> Option<serde_json::Value> {
    (!notes.is_empty()).then(|| serde_json::to_value(notes).expect("notes serialize"))
}

/// The packages `Cargo.lock` names, keyed by the name code spells for them.
pub fn lock_packages(root: &Path) -> HashMap<String, (String, u32)> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.lock")) else {
        return out;
    };
    for (i, line) in text.lines().enumerate() {
        if let Some(name) = line
            .strip_prefix("name = \"")
            .and_then(|l| l.strip_suffix('"'))
        {
            out.entry(name.replace('-', "_"))
                .or_insert((name.to_string(), i as u32 + 1));
        }
    }
    out
}

/// Derive the facts that span files from every delta of a tree. `type_checked` is true when the
/// links came from rust-analyzer: what cargo and the type-checked view establish is then
/// `resolved`.
pub fn derive(
    plan: &UnitPlan,
    lock: &HashMap<String, (String, u32)>,
    deltas: &[FileFacts],
    type_checked: bool,
) -> Assembled {
    let mut deltas: Vec<&FileFacts> = deltas.iter().collect();
    deltas.sort_by(|a, b| a.path.cmp(&b.path));
    let notes: Vec<(String, Notes)> = deltas
        .iter()
        .map(|d| (d.path.to_string_lossy().replace('\\', "/"), notes_of(d)))
        .collect();
    let items: HashMap<&str, &Item> = deltas
        .iter()
        .flat_map(|d| &d.items)
        .map(|i| (i.id.as_str(), i))
        .collect();
    let links: Vec<&arch_facts::Link> = deltas.iter().flat_map(|d| &d.links).collect();
    let tests: HashSet<&str> = notes
        .iter()
        .flat_map(|(_, n)| &n.tests)
        .map(String::as_str)
        .collect();
    let sql: BTreeSet<&SqlNote> = notes.iter().flat_map(|(_, n)| &n.sql).collect();
    let main_manifest = manifest(plan, plan.main_package);
    let mut out = Assembled::default();

    // Entries: main, routed handlers, spawned workers; the first claim on an item wins.
    let sure = if type_checked {
        Confidence::Resolved
    } else {
        Confidence::Guessed
    };
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    for d in &deltas {
        for it in d.items.iter().filter(|i| i.flags.entry) {
            entries.insert(
                it.id.clone(),
                Entry {
                    item: it.id.clone(),
                    kind: EntryKind::Main,
                    framework: None,
                    confidence: sure,
                    witness: Witness::Span {
                        file: it.file.clone(),
                        line: it.span.line,
                        col: Some(it.span.col),
                    },
                },
            );
        }
    }
    let mut routes: Vec<&RouteNote> = notes.iter().flat_map(|(_, n)| &n.routes).collect();
    routes.sort_by_key(|r| r.order);
    let mut ports: BTreeMap<String, Port> = BTreeMap::new();
    for r in routes {
        entries.entry(r.handler.clone()).or_insert(Entry {
            item: r.handler.clone(),
            kind: EntryKind::Framework,
            framework: Some(r.framework.clone()),
            confidence: Confidence::Guessed,
            witness: r.witness.clone(),
        });
        let id = format!("port:http:{}", r.name);
        ports.entry(id.clone()).or_insert(Port {
            id,
            kind: PortKind::Http,
            name: r.name.clone(),
            side: Side::Driving,
            section: Section::Unresolved,
            external: None,
            witness: r.witness.clone(),
        });
    }
    let known: BTreeSet<String> = entries.keys().cloned().collect();
    let workers = Workers {
        items: &items,
        links: &links,
        tests: &tests,
        loops: notes
            .iter()
            .flat_map(|(_, n)| &n.loops)
            .map(String::as_str)
            .collect(),
    };
    let mut spawns: Vec<&SpawnNote> = notes.iter().flat_map(|(_, n)| &n.spawns).collect();
    spawns.sort_by_key(|s| s.order);
    for (worker, witness, in_main) in workers.spawned(&spawns, &known) {
        entries.entry(worker.clone()).or_insert(Entry {
            item: worker,
            kind: EntryKind::SpawnedWorker,
            framework: None,
            // ADR 0028: resolved when the spawn is in `main`'s own body, guessed when it is
            // only reached from `main`.
            confidence: if in_main { sure } else { Confidence::Guessed },
            witness,
        });
    }

    // Tables from migrations, and who uses one as a queue.
    let users = |table: &str, pred: fn(&SqlNote) -> bool| -> Vec<String> {
        let set: BTreeSet<&str> = sql
            .iter()
            .filter(|u| u.table == table && pred(u))
            .map(|u| u.owner.as_str())
            .collect();
        set.into_iter().map(str::to_string).collect()
    };
    for (path, n) in &notes {
        for c in &n.tables {
            let dequeuers = users(&c.name, |u| u.dequeues);
            let queue = (!dequeuers.is_empty()).then(|| QueueUse {
                inserters: users(&c.name, |u| u.inserts),
                dequeuers,
            });
            out.tables.push(Table {
                name: c.name.clone(),
                witness: Witness::Declared {
                    file: path.clone(),
                    line: c.line,
                },
                queue,
            });
        }
    }
    // An inserter into a queue table queues too; that is only known once the dequeuer is.
    let queue_tables: BTreeSet<&str> = sql
        .iter()
        .filter(|u| u.dequeues)
        .map(|u| u.table.as_str())
        .collect();
    let has = |from: &str, to: &Target, kind: LinkKind| {
        links
            .iter()
            .find(|l| l.from == from && &l.to == to && l.kind == kind && l.member.is_none())
    };
    for u in sql
        .iter()
        .filter(|u| u.inserts && queue_tables.contains(u.table.as_str()))
    {
        let to = Target::Table(u.table.clone());
        let Some(reads) = has(&u.owner, &to, LinkKind::Reads) else {
            continue;
        };
        let made = |l: &&arch_facts::Link| {
            l.from == u.owner && l.to == to && l.kind == LinkKind::Queues && l.member.is_none()
        };
        if has(&u.owner, &to, LinkKind::Queues).is_some() || out.links.iter().any(|l| made(&l)) {
            continue;
        }
        out.links.push(arch_facts::Link {
            from: u.owner.clone(),
            to,
            kind: LinkKind::Queues,
            confidence: Confidence::Guessed,
            witness: reads.witness.clone(),
            flags: LinkFlags {
                origin: None,
                access: Some(Access::Write),
                compile_time: false,
                r#unsafe: false,
            },
            reason: Some(format!(
                "inserts into `{}`, which another function dequeues with `SKIP LOCKED`",
                u.table
            )),
            member: None,
        });
    }

    // Externals: crates touched, the SQL database, HTTP hosts, the terminal.
    let mut touched: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (_, n) in &notes {
        for (pkg, paths) in &n.touched {
            touched
                .entry(pkg.as_str())
                .or_default()
                .extend(paths.iter().map(String::as_str));
        }
    }
    let unit_packages: Vec<usize> = plan.unit.iter().map(|u| u.package).collect();
    for (pkg, paths) in touched {
        let dep = unit_packages.iter().find_map(|p| {
            plan.packages[*p]
                .deps
                .iter()
                .find(|d| d.package == pkg)
                .map(|d| (manifest(plan, *p), d.line))
        });
        // A crate reached only through another one is declared by the lock file.
        let locked = lock
            .values()
            .find(|(name, _)| name == pkg)
            .map(|(_, line)| ("Cargo.lock".to_string(), *line));
        let (file, line) = dep.or(locked).unwrap_or((main_manifest.clone(), 1));
        out.externals.push(External {
            id: format!("external:crate:{pkg}"),
            name: pkg.to_string(),
            kind: PortKind::Crate,
            touched: paths.into_iter().map(str::to_string).collect(),
            witness: Witness::Declared { file, line },
        });
    }
    let sqlx = unit_packages.iter().find_map(|p| {
        plan.packages[*p]
            .deps
            .iter()
            .find(|d| d.package == "sqlx")
            .map(|d| (manifest(plan, *p), d.clone()))
    });
    let touched_tables: BTreeSet<&str> = sql.iter().map(|u| u.table.as_str()).collect();
    if !touched_tables.is_empty() || (!out.tables.is_empty() && sqlx.is_some()) {
        let db = sqlx
            .as_ref()
            .and_then(|(_, d)| {
                ["postgres", "mysql", "sqlite"]
                    .into_iter()
                    .find(|f| d.features.iter().any(|x| x == f))
            })
            .unwrap_or("sql");
        let witness = match (&sqlx, out.tables.first()) {
            (Some((file, d)), _) => Witness::Declared {
                file: file.clone(),
                line: d.line,
            },
            (None, Some(t)) => t.witness.clone(),
            (None, None) => Witness::Declared {
                file: main_manifest.clone(),
                line: 1,
            },
        };
        let id = format!("external:sql:{db}");
        out.externals.push(External {
            id: id.clone(),
            name: db.to_string(),
            kind: PortKind::Sql,
            touched: touched_tables.into_iter().map(str::to_string).collect(),
            witness: witness.clone(),
        });
        ports.insert(
            format!("port:sql:{db}"),
            Port {
                id: format!("port:sql:{db}"),
                kind: PortKind::Sql,
                name: db.to_string(),
                side: Side::Driven,
                section: Section::DataStores,
                external: Some(id),
                witness,
            },
        );
    }
    let mut hosts: BTreeMap<&str, &HostNote> = BTreeMap::new();
    for h in notes.iter().flat_map(|(_, n)| &n.hosts) {
        let first = hosts.entry(h.host.as_str()).or_insert(h);
        if h.order < first.order {
            *first = h;
        }
    }
    for (host, h) in hosts {
        let id = format!("external:http:{host}");
        out.externals.push(External {
            id: id.clone(),
            name: host.to_string(),
            kind: PortKind::Http,
            touched: vec![],
            witness: h.witness.clone(),
        });
        ports.insert(
            format!("port:http:{host}"),
            Port {
                id: format!("port:http:{host}"),
                kind: PortKind::Http,
                name: host.to_string(),
                side: Side::Driven,
                section: Section::ThirdParty,
                external: Some(id),
                witness: h.witness.clone(),
            },
        );
    }
    if let Some((_, w)) = notes
        .iter()
        .filter_map(|(_, n)| n.tty.as_ref())
        .min_by_key(|(o, _)| *o)
    {
        let id = "external:tty:log".to_string();
        out.externals.push(External {
            id: id.clone(),
            name: "log".into(),
            kind: PortKind::Tty,
            touched: vec![],
            witness: w.clone(),
        });
        ports.insert(
            "port:tty:log".into(),
            Port {
                id: "port:tty:log".into(),
                kind: PortKind::Tty,
                name: "log".into(),
                side: Side::Driven,
                section: Section::Os,
                external: Some(id),
                witness: w.clone(),
            },
        );
    }
    out.entries = entries.into_values().collect();
    out.ports = ports.into_values().collect();
    out
}

fn manifest(plan: &UnitPlan, package: usize) -> String {
    plan.packages[package]
        .manifest
        .to_string_lossy()
        .replace('\\', "/")
}

/// ADR 0028: the functions a start-up spawn runs forever are entries.
struct Workers<'a> {
    items: &'a HashMap<&'a str, &'a Item>,
    links: &'a [&'a arch_facts::Link],
    tests: &'a HashSet<&'a str>,
    loops: HashSet<&'a str>,
}

impl Workers<'_> {
    /// Spawned workers, with the spawn's witness and whether it sits in `main` itself.
    fn spawned(
        &self,
        spawns: &[&SpawnNote],
        entries: &BTreeSet<String>,
    ) -> Vec<(String, Witness, bool)> {
        let mut calls: HashMap<&str, Vec<&str>> = HashMap::new();
        for l in self.links {
            if l.kind == LinkKind::Calls
                && let Target::Item(to) = &l.to
                && self.items.contains_key(l.from.as_str())
                && self.items.contains_key(to.as_str())
            {
                calls.entry(l.from.as_str()).or_default().push(to.as_str());
            }
        }
        let top = |id: &str| -> String {
            self.items[id]
                .module
                .split("::")
                .next()
                .unwrap_or("")
                .to_string()
        };
        let reach = |from: &[&str], within: Option<&str>| {
            let mut seen: BTreeSet<String> = from.iter().map(|s| s.to_string()).collect();
            let mut queue: Vec<String> = seen.iter().cloned().collect();
            while let Some(x) = queue.pop() {
                for y in calls.get(x.as_str()).into_iter().flatten() {
                    if within.is_none_or(|w| w == top(y)) && seen.insert(y.to_string()) {
                        queue.push(y.to_string());
                    }
                }
            }
            seen
        };
        let mains: Vec<&str> = self
            .items
            .values()
            .filter(|i| i.flags.entry && !self.tests.contains(i.id.as_str()))
            .map(|i| i.id.as_str())
            .collect();
        let from_main = reach(&mains, None);
        let non_test: Vec<&str> = entries
            .iter()
            .map(String::as_str)
            .filter(|e| !self.tests.contains(e))
            .collect();
        let from_entries = reach(&non_test, None);
        let test_code = |id: &str| {
            self.tests.contains(id)
                || self.items[id]
                    .flags
                    .cfg
                    .as_deref()
                    .is_some_and(|c| c.contains("test"))
        };
        let mut out: Vec<(String, Witness, bool)> = Vec::new();
        for s in spawns {
            let (owner, target) = (s.owner.as_str(), s.target.as_str());
            if !self.items.contains_key(owner) || !self.items.contains_key(target) {
                continue;
            }
            if s.in_loop || !from_main.contains(owner) || test_code(owner) || test_code(target) {
                continue;
            }
            // §3: nothing calls it directly from an entry (entries themselves are reached by nobody).
            if from_entries.contains(target) && !non_test.contains(&target) {
                continue;
            }
            let loops = reach(&[target], Some(&top(target)))
                .iter()
                .any(|f| self.loops.contains(f.as_str()));
            if loops && !out.iter().any(|(t, _, _)| t == target) {
                out.push((
                    target.to_string(),
                    s.witness.clone(),
                    mains.contains(&owner),
                ));
            }
        }
        out
    }
}
