//! `arch-session` · the state machine, plan, scheduler, gate runner, judge, context pack,
//! thread, resolve element.
//!
//! Depends on facts, views, driver and forge. Milestone 5 of `handoff-arch.md` (issue #47).
//! Present today: [`machine`] (the one transition table over [`arch_facts::SessionState`]),
//! [`shaper`] (the core shaper: groups by dependency, one gate per group) and [`accept()`]
//! (branch, worktree, `.arch/sessions/<id>/`, one commit). The session record
//! the scheduler resumes from is [`arch_facts::Session`], `.arch/sessions/<id>/session.toml`.

pub mod accept;
pub mod machine;
pub mod shaper;

pub use accept::{AcceptOptions, accept};
pub use machine::{Trigger, next};
pub use shaper::shape;

use std::path::PathBuf;

use arch_facts::{SessionId, SessionState};

/// Errors crossing this crate's boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Accept on a session with no plan draft in the cache.
    #[error("session {0}: no plan draft in the cache")]
    NoDraft(SessionId),
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
