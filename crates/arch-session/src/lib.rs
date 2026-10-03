//! `arch-session` · the state machine, plan, scheduler, gate runner, judge, context pack,
//! thread, resolve element.
//!
//! Depends on facts, views, driver and forge. Milestone 5 of `handoff-arch.md`; a skeleton
//! in milestone 1. State values are [`arch_facts::SessionState`].

pub mod pack;

/// Errors crossing this crate's boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An `.arch/` file or the store failed.
    #[error(transparent)]
    Facts(#[from] arch_facts::Error),
    /// A view failed.
    #[error(transparent)]
    Views(#[from] arch_views::Error),
}
