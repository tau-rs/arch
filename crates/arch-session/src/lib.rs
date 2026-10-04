//! `arch-session` · the state machine, plan, scheduler, gate runner, judge, context pack,
//! thread, resolve element.
//!
//! Depends on facts, views, driver and forge. Milestone 5 of `handoff-arch.md` (issue #47).
//! - [`machine`]: the one transition table over [`arch_facts::SessionState`];
//! - [`shaper`]: the core shaper, groups by dependency, one gate per group;
//! - [`accept()`]: branch, worktree, `.arch/sessions/<id>/`, one commit;
//! - [`Engine`]: the scheduler, the element runs, the gate ([`gate`]), the judge ([`judge`]),
//!   fix rounds and the four doors, asks and deviations ([`denial`]);
//! - [`pack`]: the context pack (ADR 0005).
//!
//! `arch check` and the facts need the analyzer, which this crate may not depend on: they come
//! through the [`Project`] port, implemented by `arch-api`. The session record
//! the scheduler resumes from is [`arch_facts::Session`], `.arch/sessions/<id>/session.toml`.

pub mod accept;
pub mod denial;
pub mod engine;
pub mod gate;
pub mod judge;
pub mod machine;
pub mod pack;
pub mod planner;
pub mod review;
pub mod shaper;

pub use accept::{AcceptOptions, accept};
pub use engine::{Door, Engine, EngineOptions, Typology};
pub use machine::{Trigger, next};
pub use shaper::shape;

use std::path::PathBuf;

use std::path::Path;

use arch_facts::{ElementId, Facts, SessionId, SessionState};

pub(crate) use accept::git;

/// What the engine needs from the rest of arch, implemented by `arch-api`.
pub trait Project {
    /// `arch check` on the worktree: the `schemas/check.schema.json` document.
    fn check(&self, worktree: &Path) -> Result<serde_json::Value, String>;

    /// The worktree's facts, for the context pack.
    fn facts(&self, worktree: &Path) -> Result<Facts, String>;
}

/// Errors crossing this crate's boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Accept on a session with no plan draft in the cache.
    #[error("session {0}: no plan draft in the cache")]
    NoDraft(SessionId),
    /// No `session.toml` or `plan.toml` for the session in the worktree.
    #[error("session {0}: no session.toml and plan.toml in this worktree")]
    NoSession(SessionId),
    /// The plan has no such element.
    #[error("plan: no element {0}")]
    NoElement(ElementId),
    /// The cursor points past the plan's groups.
    #[error("plan: no group {0}")]
    NoGroup(usize),
    /// The session waits on nothing, though its state says it does.
    #[error("session {0}: in asks or deviation with no element waiting")]
    NotWaiting(SessionId),
    /// The plan could not be had: a bad `plan.toml`, or a planner that gave no elements.
    #[error("plan: {0}")]
    Plan(String),
    /// The forge failed or refused.
    #[error(transparent)]
    Forge(#[from] arch_forge::ForgeError),
    /// The session's branch has no request on the forge.
    #[error("session {0}: no {1} on the forge for its branch; open it with `arch session pr`")]
    NoRequest(SessionId, &'static str),
    /// The request was closed without merging.
    #[error("{0} was closed without merging")]
    RequestClosed(String),
    /// The request's checks failed or are still running.
    #[error("{what} {state}: arch merges once they pass")]
    ChecksNotGreen {
        /// The forge's word for its checks.
        what: &'static str,
        /// `failed` or `pending`.
        state: &'static str,
    },
    /// The merge strategy is not one the repo allows, or the repo allows several and none was
    /// named.
    #[error("{0}")]
    Strategy(String),
    /// A door that a later issue opens.
    #[error("{what} is not there yet ({issue})")]
    NotYet {
        /// The door.
        what: &'static str,
        /// The issue that brings it.
        issue: &'static str,
    },
    /// The driver failed.
    #[error(transparent)]
    Driver(#[from] arch_driver::DriverError),
    /// The tool layer's files could not be written.
    #[error(transparent)]
    ToolLayer(#[from] arch_driver::tool_layer::ToolLayerError),
    /// Git answered with an error.
    #[error("{0}")]
    Git(String),
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// An `.arch/` file or the store failed.
    #[error(transparent)]
    Facts(#[from] arch_facts::Error),
    /// A view failed.
    #[error(transparent)]
    Views(#[from] arch_views::Error),
    /// A trigger that has no edge from the session's state.
    #[error("a session in state {from:?} cannot take {trigger:?}")]
    IllegalTransition {
        /// The state the session is in.
        from: SessionState,
        /// What was asked of it.
        trigger: Trigger,
    },
    /// An element depends on an element the plan does not hold.
    #[error("plan: {element} depends on {depends_on}, which is not in the plan")]
    UnknownDependency {
        /// The element's label.
        element: String,
        /// The missing id.
        depends_on: String,
    },
    /// The elements' dependencies form a cycle, so no group can go first.
    #[error("plan: dependency cycle among {}", .elements.join(", "))]
    Cycle {
        /// The labels of the elements left unordered.
        elements: Vec<String>,
    },
}
