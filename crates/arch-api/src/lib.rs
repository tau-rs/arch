//! `arch-api` · one method set (views, intents, sessions, Ask, settings) served over
//! JSON-RPC on a local socket and over MCP, same handlers.
//!
//! Depends on every other library crate. Milestone 6 of `handoff-arch.md` brings the two
//! transports. Present today, for the CLI (milestone 4, issue #5): [`init()`] and [`check()`],
//! the two methods `arch init` and `arch check` call. The CLI depends on this crate only, so the
//! types it prints are re-exported here.

use std::path::PathBuf;

pub mod check;
pub mod init;

pub use arch_facts::{Areas, Column, ColumnRule, Confidence, Level, LinkKind, Witness};
pub use arch_views::{Allowed, Finding};
pub use check::{CHECK_SCHEMA_VERSION, CheckOutput, Summary, check};
pub use init::{InitOptions, InitOutcome, init};

/// Errors crossing the API boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `arch check` on a repository without `.arch/`.
    #[error("{0}: no .arch/areas.toml or .arch/rules here; run `arch init` first")]
    NotInitialized(PathBuf),
    /// `arch init` on a repository that already has `.arch/` files.
    #[error(
        "{0}: .arch/ already holds areas.toml or rules; `arch init` runs once, edit the files instead"
    )]
    AlreadyInitialized(PathBuf),
    /// The analyzer failed.
    #[error(transparent)]
    Analyze(#[from] arch_analyze::Error),
    /// An `.arch/` file or the store failed.
    #[error(transparent)]
    Facts(#[from] arch_facts::Error),
    /// A view failed.
    #[error(transparent)]
    Views(#[from] arch_views::Error),
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// Git answered with an error.
    #[error("git: {0}")]
    Git(String),
}
