//! `arch-api` · one method set (views, intents, sessions, Ask, settings) served over
//! JSON-RPC on a local socket and over MCP, same handlers.
//!
//! Depends on every other library crate. Milestone 6 of `handoff-arch.md` brings the two
//! transports. Present today, for the CLI: [`init()`] and [`check()`] (milestone 4, issue #5),
//! and [`hook()`] and [`mcp()`], the tool layer an agent runs under (ADR 0012, issue #46). The
//! app method set and its published schema, `schemas/arch-api.json`, are in [`rpc`] (ADR 0034,
//! issue #7), served by [`serve()`]. The CLI depends on this crate only, so the types it prints are re-exported here.

use std::path::PathBuf;

pub mod check;
pub mod init;
pub mod rpc;
#[cfg(unix)]
pub mod serve;
pub mod tool_layer;

pub use arch_facts::{Areas, Column, ColumnRule, Confidence, Level, LinkKind, Witness};
pub use arch_views::{Allowed, Finding};
pub use check::{CHECK_SCHEMA_VERSION, CheckOutput, Summary, check};
pub use init::{InitOptions, InitOutcome, init};
pub use tool_layer::{
    ArchProject, CLAUDE_CO_AUTHOR, HookOutcome, Phase, ToolLayerTarget, hook, mcp,
};

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
    /// The tool layer (`arch hook`, `arch mcp`) failed.
    #[error(transparent)]
    ToolLayer(#[from] arch_driver::tool_layer::ToolLayerError),
    /// `arch serve` could not start or stopped.
    #[cfg(unix)]
    #[error(transparent)]
    Serve(#[from] serve::ServeError),
    /// `arch serve` has no named-pipe transport yet (ADR 0034 §5).
    #[error("arch serve is not supported on this platform yet")]
    Unsupported,
}

/// `arch serve` for the repo at `repo`: bind its socket, say where on stderr, serve until killed.
pub fn serve(repo: &std::path::Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        let server = serve::bind(repo, &serve::SocketEnv::current())?;
        eprintln!("arch serve: listening on {}", server.path().display());
        Ok(server.run()?)
    }
    #[cfg(not(unix))]
    {
        let _ = repo;
        Err(Error::Unsupported)
    }
}
