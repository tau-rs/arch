//! Facts for one repository at one commit: the `facts.json` document.
//!
//! Golden-fact stability rules (arch-fixtures compares these files byte for byte):
//! every array is sorted by its first field (`id`, or `from` then `to` then `kind` for links);
//! ids never contain line numbers; optional fields are omitted when empty.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version of this document shape. `0` until the first golden facts are pinned; bumped on any
/// change that breaks a reader, announced in arch-design before shipping (HANDOFF §5).
pub const SCHEMA_VERSION: u32 = 0;

/// Everything the analyzer knows about one repository at one commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    /// Shape version of this document; see [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The repository and commit the facts describe.
    pub repo: Repo,
    /// Who produced the facts and what it could not type-check.
    pub analyzer: Analyzer,
    /// Cargo packages seen, analyzed or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crates: Vec<Crate>,
    /// Code items: functions, types, traits, impls, modules, macros, consts, statics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// Relations between items, ports, externals and tables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// The unit's ports: where the outside world touches it and where it touches the world.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Things outside the repository the code talks to, with the part of their surface touched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub externals: Vec<External>,
    /// Entry points, including framework-held ones (a handler axum calls is an entry).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// Database tables named in `migrations/*.sql`, with queue use when detected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
    /// Commits on the branch as facts (author, trailers, files, plan element).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<Commit>,
}

/// The repository and the commit (or worktree state) the facts belong to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    /// Repository name as the forge knows it, e.g. `zero2prod`.
    pub name: String,
    /// Full commit hash, or a worktree-state hash prefixed `wt:` for uncommitted trees (ADR 2).
    pub commit: String,
    /// The one unit analyzed in V1: the main bin (or lib) and its closure (ADR 7).
    pub unit: Unit,
}

/// The analyzed unit. V1 has exactly one per repository; the id is the scope on every item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    /// Scope id carried by every item, e.g. `unit:zero2prod`.
    pub id: String,
    /// The cargo target the unit is built from: first `[[bin]]`, else the lib, or the
    /// `areas.toml` override.
    pub main_target: String,
}

/// The producer of the facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Analyzer {
    /// `arch-analyze`.
    pub name: String,
    /// Analyzer version (crate version plus the rust-analyzer crates' version).
    pub version: String,
    /// Crates that fell back to syntax-only facts, with the reason (ADR 10).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded: Vec<Degraded>,
}

/// One crate rust-analyzer could not type-check; all its facts are `guessed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Degraded {
    /// Crate name.
    #[serde(rename = "crate")]
    pub crate_name: String,
    /// Why, as the tool reported it.
    pub reason: String,
}

/// A cargo package in the workspace or its closure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Crate {
    /// Crate name.
    pub name: String,
    /// Whether it is part of the unit or marked "not analyzed" with a reason.
    pub status: CrateStatus,
    /// Targets by kind, e.g. `lib`, `bin:zero2prod`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
}

/// Why a crate is or is not in the unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "reason")]
pub enum CrateStatus {
    /// In the unit's closure; facts produced.
    Analyzed,
    /// A tool bin, an example, or a target outside the closure; named in the status line only.
    NotAnalyzed(String),
}

/// A code item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Stable id: `<crate>::<module path>::<name>` plus `#<kind>`; impls are
    /// `<crate>::<module>::impl <Trait> for <Type>#impl` (or `impl <Type>#impl`), with `#impl.2`
    /// for a second block in the same module. Never contains a line number.
    pub id: String,
    /// What kind of item.
    pub kind: ItemKind,
    /// Bare name (`subscribe`, `NewSubscriber`).
    pub name: String,
    /// Repository-relative path, forward slashes.
    pub file: String,
    /// Where the item is in the file.
    pub span: Span,
    /// Crate the item belongs to.
    #[serde(rename = "crate")]
    pub crate_name: String,
    /// Module path inside the crate, `::`-separated, empty for the crate root.
    pub module: String,
    /// Visibility as declared.
    pub visibility: Visibility,
    /// Reachable through a `pub use` somewhere in the crate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reexported: bool,
    /// Markers used by the map and the lints.
    #[serde(default, skip_serializing_if = "ItemFlags::is_empty")]
    pub flags: ItemFlags,
    /// Scope id; V1: the unit's id (present so V2 units are an addition, spec §3).
    pub scope: String,
}

/// Item kinds (HANDOFF §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// A function or method.
    Fn,
    /// A struct.
    Struct,
    /// An enum.
    Enum,
    /// A trait.
    Trait,
    /// An impl block.
    Impl,
    /// A module.
    Mod,
    /// A macro.
    Macro,
    /// A const.
    Const,
    /// A static.
    Static,
    /// A type alias.
    TypeAlias,
    /// A union.
    Union,
}

/// Declared visibility, folded to five levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    /// `pub`.
    Pub,
    /// `pub(crate)`.
    Crate,
    /// `pub(super)`.
    Super,
    /// `pub(in path)`.
    Restricted,
    /// No modifier.
    Private,
}

/// Item markers. All optional; absent means false.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemFlags {
    /// An entry point (also listed in [`Facts::entries`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub entry: bool,
    /// Contains or is an `unsafe` block, fn, impl or trait.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub r#unsafe: bool,
    /// Implements `Drop`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drop: bool,
    /// Produced by a macro expansion or a build script.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub generated: bool,
    /// The `cfg` predicate guarding the item, when any (`test`, `feature = "x"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<String>,
}

impl ItemFlags {
    /// True when no flag is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A position range in a file; lines and columns are 1-based, `end` inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    /// First line.
    pub line: u32,
    /// Column on the first line.
    pub col: u32,
    /// Last line.
    pub end_line: u32,
    /// Column on the last line.
    pub end_col: u32,
}

/// A relation. `from` is always an item; `to` is an item, a port, an external or a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Source item id.
    pub from: String,
    /// Target.
    pub to: Target,
    /// One of the 21 kinds in four families, or `refers-to` (FINDINGS F-3, issue #10).
    pub kind: LinkKind,
    /// How sure the analyzer is (ADR 9).
    pub confidence: Confidence,
    /// Where the relation is visible.
    pub witness: Witness,
    /// Qualifiers.
    #[serde(default, skip_serializing_if = "LinkFlags::is_empty")]
    pub flags: LinkFlags,
    /// For `guessed` links: the pattern that produced the guess; for unresolved targets: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// An item id.
    Item(String),
    /// A port id.
    Port(String),
    /// An external id.
    External(String),
    /// A table name.
    Table(String),
}

/// Link kinds. Families: call · type · data · structure, plus `refers-to` outside them.
/// `resolved` by default from the type-checked view; `hands-off`, `listens-to`, `calls-out` and
/// `wires` are pattern guesses (HANDOFF §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    // call family
    /// Calls a function or method.
    Calls,
    /// Calls through a port (a trait object or generic the unit owns).
    CallsPort,
    /// Takes a port as a dependency (field, parameter) without a call site in this item.
    DependsOnPort,
    /// Hands work to another task, thread or channel (spawn, send).
    HandsOff,
    /// Receives from a channel, topic or signal.
    ListensTo,
    /// Calls out of the unit to an external.
    CallsOut,
    /// Wires an implementation to a port at composition time (main, a builder, a container).
    Wires,
    /// A registered `route → handler`, with the middleware stack in `reason`-free `witness` lines.
    Routes,
    // type family
    /// Implements a trait.
    Implements,
    /// A trait refines (has as supertrait) another.
    Refines,
    /// Inherits in the derive/delegation sense (`#[derive]`, `Deref` delegation).
    Inherits,
    /// Uses a type in a signature or a body.
    UsesType,
    /// Holds a value of the type (a field).
    Holds,
    /// Constructs the type.
    Constructs,
    /// Matches on the type's variants.
    MatchesOn,
    // data family
    /// Reads or writes state, a field, a table; the direction is `flags.access`.
    Reads,
    /// Uses a shared table as a queue; inserter or dequeuer per `flags.access`.
    Queues,
    // structure family
    /// Tests the target.
    Tests,
    /// Re-exports the target (`pub use`).
    ReExports,
    /// Expands to the target (macro).
    Expands,
    /// Decorates the target (attribute macro).
    Decorates,
    // outside the families
    /// Names the target without a stronger relation known.
    RefersTo,
}

impl LinkKind {
    /// The family the kind belongs to; `None` for `refers-to`.
    pub fn family(self) -> Option<LinkFamily> {
        use LinkKind::*;
        Some(match self {
            Calls | CallsPort | DependsOnPort | HandsOff | ListensTo | CallsOut | Wires
            | Routes => LinkFamily::Call,
            Implements | Refines | Inherits | UsesType | Holds | Constructs | MatchesOn => {
                LinkFamily::Type
            }
            Reads | Queues => LinkFamily::Data,
            Tests | ReExports | Expands | Decorates => LinkFamily::Structure,
            RefersTo => return None,
        })
    }

    /// Every kind, in schema order.
    pub const ALL: [LinkKind; 22] = {
        use LinkKind::*;
        [
            Calls,
            CallsPort,
            DependsOnPort,
            HandsOff,
            ListensTo,
            CallsOut,
            Wires,
            Routes,
            Implements,
            Refines,
            Inherits,
            UsesType,
            Holds,
            Constructs,
            MatchesOn,
            Reads,
            Queues,
            Tests,
            ReExports,
            Expands,
            Decorates,
            RefersTo,
        ]
    };
}

/// The four link families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LinkFamily {
    /// Control goes from `from` to `to`.
    Call,
    /// `from` depends on the shape of `to`.
    Type,
    /// `from` touches data owned by `to`.
    Data,
    /// Source structure: tests, re-exports, macros.
    Structure,
}

/// How sure a fact is (ADR 9). A finding on a `guessed` link warns, never blocks; on `declared`
/// it cannot gate a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// From the type-checked view.
    Resolved,
    /// From a pattern, or from the syntax-only fallback.
    Guessed,
    /// Written by a person in `.arch/` (a rule, an allow, an area).
    Declared,
}

/// Where a fact can be seen (the witness rule, spec §1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Witness {
    /// A location in a source file of the repository.
    Span {
        /// Repository-relative path.
        file: String,
        /// 1-based line.
        line: u32,
        /// 1-based column, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        col: Option<u32>,
    },
    /// A contract or a declaration (`Cargo.toml`, `.arch/rules`, a migration file).
    Declared {
        /// Repository-relative path.
        file: String,
        /// 1-based line.
        line: u32,
    },
    /// A tool's output (`cargo metadata`, a test run), identified by its content hash.
    Tool {
        /// Tool name and arguments as run.
        tool: String,
        /// SHA-256 of the output, hex.
        output_sha256: String,
    },
}

/// Link qualifiers (HANDOFF §2: declared-or-shape · read-or-write · compile-time · unsafe).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkFlags {
    /// Whether the relation was declared (an explicit `impl`, `use`) or inferred from shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// Read or write, for `reads` and `queues`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
    /// Happens at compile time (macro, const eval, build script).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compile_time: bool,
    /// Crosses an `unsafe` boundary.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub r#unsafe: bool,
}

impl LinkFlags {
    /// True when no flag is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Declared or inferred from shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// Written in source.
    Declared,
    /// Inferred from structure.
    Shape,
}

/// Read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    /// Reads.
    Read,
    /// Writes (inserts, updates, sends).
    Write,
}

/// A port: a place on the unit's flat sides where the world touches it or it touches the world
/// (spec §5, MAP-31/32).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// Stable id, e.g. `port:http:POST /subscriptions`, `port:sql:postgres`.
    pub id: String,
    /// Protocol or medium.
    pub kind: PortKind,
    /// Display name.
    pub name: String,
    /// Driving (the world calls the unit) or driven (the unit calls the world).
    pub side: Side,
    /// Section on the side, in the map's fixed order.
    pub section: Section,
    /// The external behind a driven port, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    /// Where the port is declared or first used.
    pub witness: Witness,
}

/// Port kinds (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PortKind {
    /// gRPC, JSON-RPC and the like.
    Rpc,
    /// HTTP routes and clients.
    Http,
    /// Command-line arguments and subcommands.
    Cli,
    /// Message topics and channels.
    Topic,
    /// A dependency crate used as a boundary.
    Crate,
    /// SQL databases.
    Sql,
    /// The crate's own public API (`pub` surface).
    Pub,
    /// Redis.
    Redis,
    /// The filesystem.
    Fs,
    /// The terminal.
    Tty,
    /// Declared by a person in `.arch/`.
    Declared,
}

/// Which side of the unit a port sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    /// The world calls the unit.
    Driving,
    /// The unit calls the world.
    Driven,
}

/// Sections on a side, fixed order (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    /// Platform services (own infrastructure).
    PlatformServices,
    /// Third-party services.
    ThirdParty,
    /// Events and topics.
    Events,
    /// Data stores.
    DataStores,
    /// The operating system.
    Os,
    /// Libraries.
    Libraries,
    /// Could not be placed.
    Unresolved,
}

/// Something outside the repository the code talks to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct External {
    /// Stable id, e.g. `external:crate:sqlx`, `external:http:api.postmarkapp.com`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Medium.
    pub kind: PortKind,
    /// The subset of its surface the unit touches: function paths, endpoints, table names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<String>,
    /// Where the dependency is declared.
    pub witness: Witness,
}

/// An entry point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The item.
    pub item: String,
    /// The framework that holds the entry; absent for a plain `main`, test or bench.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<Framework>,
    /// Why it is an entry.
    pub witness: Witness,
}

/// Frameworks that hold entries (spec §7 table), plus the generic rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Framework {
    /// axum.
    Axum,
    /// actix-web.
    Actix,
    /// tonic.
    Tonic,
    /// bevy `App` systems.
    Bevy,
    /// AWS Lambda.
    Lambda,
    /// Generic rule: the entry calls into a dependency that calls back; the dependency is named.
    Generic(String),
}

/// A database table named in `migrations/*.sql`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Table name.
    pub name: String,
    /// The migration that creates it.
    pub witness: Witness,
    /// Set when the table is used as a queue (spec §7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueUse>,
}

/// Who inserts and who dequeues from a shared table used as a queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueUse {
    /// Item ids that insert.
    pub inserters: Vec<String>,
    /// Item ids that dequeue.
    pub dequeuers: Vec<String>,
}

/// A commit as a fact (spec §7, ADR 16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    /// Full hash.
    pub hash: String,
    /// Author as `Name <email>`.
    pub author: String,
    /// First line of the message.
    pub summary: String,
    /// Trailers, in order; keys may repeat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trailers: Vec<Trailer>,
    /// Files touched, repository-relative.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The plan element the commit realizes, from the `Arch-Element` trailer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
}

/// One commit trailer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trailer {
    /// Key, e.g. `Arch-Element`.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Facts grouped by file: the per-file delta the store keys by content hash (ADR 2).
pub type ByFile<T> = BTreeMap<String, Vec<T>>;

impl Facts {
    /// An empty document for the given repository and analyzer, at the current schema version.
    pub fn empty(repo: Repo, analyzer: Analyzer) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            repo,
            analyzer,
            crates: Vec::new(),
            items: Vec::new(),
            links: Vec::new(),
            ports: Vec::new(),
            externals: Vec::new(),
            entries: Vec::new(),
            tables: Vec::new(),
            commits: Vec::new(),
        }
    }

    /// The JSON Schema (draft 2020-12) for this document, as published in `schemas/facts.schema.json`.
    pub fn json_schema() -> serde_json::Value {
        let mut schema = schemars::schema_for!(Facts);
        schema.insert(
            "$id".into(),
            "https://github.com/tau-rs/arch/blob/main/schemas/facts.schema.json".into(),
        );
        schema.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_one_kinds_in_four_families_plus_refers_to() {
        let in_families = LinkKind::ALL
            .iter()
            .filter(|k| k.family().is_some())
            .count();
        assert_eq!(in_families, 21);
        assert_eq!(LinkKind::RefersTo.family(), None);
        assert_eq!(LinkKind::ALL.len(), 22);
    }

    #[test]
    fn empty_flags_are_omitted() {
        let v = serde_json::to_value(ItemFlags::default()).unwrap();
        assert_eq!(v, serde_json::json!({}));
        let v = serde_json::to_value(LinkFlags::default()).unwrap();
        assert_eq!(v, serde_json::json!({}));
    }
}
