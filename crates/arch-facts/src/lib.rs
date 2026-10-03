//! `arch-facts` · the model, the sqlite store, the `.arch/` formats and the event types.
//!
//! The one crate every other arch crate depends on, and the only one that opens the
//! database (ADR 0001). It decides nothing: every shape here traces to
//! `arch-design/spec/arch-v1-spec.md` §7 or to an ADR by number; open questions are filed in
//! arch-design as `from:arch` issues (see `docs/arch-facts.md`).
//!
//! - [`model`]: the fact model; it owns `schemas/facts.schema.json`, generated from the types
//!   (`ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts` regenerates; CI fails on drift).
//! - [`session`]: sessions, plans and elements, records, thread entries.
//! - [`hash`]: content hashes and tree keys (commit hash or worktree-state hash, ADR 0002).
//! - [`store`]: sqlite under `.arch/cache/` (ADR 0001, 0002, 0020).
//! - [`arch_dir`]: readers and writers for the committed `.arch/` files (spec §7; ADR 0003,
//!   0004, 0021) and the `refs/notes/arch` archive.
//! - [`event`]: the event types for the bus.

pub mod arch_dir;
pub mod error;
pub mod event;
pub mod hash;
pub mod model;
pub mod session;
pub mod store;

pub use arch_dir::{
    Allow, Allows, ArchDir, Archive, ArchiveFile, AreaOverride, Areas, AttributedWrite, Column,
    ColumnRule, ExpectedWrite, Level, LintLevel, LintSetting, NOTES_REF, Rule, Rules, SessionDir,
    ToolLayerFile, ToolLayerState, tool_layer_dir, tool_layer_states,
};
pub use error::{Error, Result};
pub use event::{Attribution, Event, SubsystemState};
pub use hash::{ContentHash, TreeKey};
pub use model::*;
pub use session::*;
pub use store::{Assembled, FileFacts, Store, TreeHead};
