//! Event types for the bus (`handoff-arch.md` §2; spec §6 Sync; ADR 0012, 0023).
//!
//! The watcher, the tool layer, the scheduler and the views publish these; the API relays
//! them to the app and to What's new.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hash::TreeKey;
use crate::session::{ElementId, Question, SessionId, SessionState};

/// Who made a write (spec §6 Sync; ADR 0012): writes through arch's tool layer are tagged
/// so they do not loop; an unattributed write is a `you` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "kebab-case")]
pub enum Attribution {
    /// You, or any unattributed writer.
    You,
    /// A session's sub-agent, through the tool layer.
    Session {
        /// The session.
        session: SessionId,
        /// The element, when the write was within one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element: Option<ElementId>,
    },
    /// arch itself (Keep writing `areas.toml`, a session folder).
    Arch,
}

/// A subsystem's status-bar state (ADR 0023): never a modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubsystemState {
    /// Working.
    Ok,
    /// Working with reduced facts (ADR 0010).
    Degraded,
    /// Failed; the last good view is kept.
    Error,
}

/// An event on the bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// Files changed in a worktree (the watcher, debounced).
    FilesChanged {
        /// The worktree.
        worktree: PathBuf,
        /// The files, relative to the worktree.
        files: Vec<PathBuf>,
        /// Who.
        attribution: Attribution,
    },
    /// Facts were recomputed for files of a tree.
    FactsUpdated {
        /// The tree.
        tree: TreeKey,
        /// The files recomputed.
        files: Vec<PathBuf>,
    },
    /// Findings re-ran on a branch's diff.
    FindingsUpdated {
        /// The branch.
        branch: String,
        /// New findings.
        added: u32,
        /// Fixed findings.
        fixed: u32,
    },
    /// Items derivation could not place (spec §6 Sync, the Unplaced tray).
    Unplaced {
        /// The items.
        items: Vec<String>,
    },
    /// A session changed state.
    SessionState {
        /// The session.
        session: SessionId,
        /// The new state.
        state: SessionState,
    },
    /// A session asks (spec §6 Asks).
    Ask {
        /// The session.
        session: SessionId,
        /// The element asking, when any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element: Option<ElementId>,
        /// The questions.
        questions: Vec<Question>,
    },
    /// A write was denied by the tool layer (ADR 0012).
    Denied {
        /// The session.
        session: SessionId,
        /// The element.
        element: ElementId,
        /// The path.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// What's new has new lines for a branch (spec §4, §6).
    WhatsNew {
        /// The branch.
        branch: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// A subsystem's state changed (ADR 0023).
    Subsystem {
        /// Which: `analyzer`, `forge`, `driver`, `store`.
        name: String,
        /// State.
        state: SubsystemState,
        /// Reason, shown on the Checks tab.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}
