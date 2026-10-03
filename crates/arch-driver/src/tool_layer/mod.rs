//! The tool layer (ADR 0012): the only confinement an agent has in V1.
//!
//! Claude Code calls `arch hook pre` before every write and `arch hook post` after it, and
//! arch hands the agent an MCP server with four tools: `read`, `check`, `commit`, `ask`. All of
//! them work on one element of one session, in that session's worktree ([`Scope`]).
//!
//! ```text
//! agent ──Edit──▶ arch hook pre ── stale-write guard ─▶ element-scope veto ─▶ allow (expect hash)
//!                                    └─ deny: exit 2 + reason, Denial record ◀─┘
//!       ◀─done── arch hook post ── confirm the hash on disk, attribute the write
//! ```
//!
//! - [`guard`]: the pure decisions over (hook input, state, element, file on disk).
//! - [`hook`]: `arch hook pre|post`, around the guard: stdin, state file, Denial record.
//! - [`mcp`]: `arch mcp`, JSON-RPC 2.0 over stdio.
//! - [`commit`]: the commit tool (ADR 0016).
//! - [`config`]: the `--settings` and `--mcp-config` files the claude-code driver is given.
//!
//! `check()` and the commit's area need the analyzer and the views, which this crate may not
//! depend on: they come through the [`Project`] port, implemented by `arch-api`.

pub mod commit;
pub mod config;
pub mod guard;
pub mod hook;
pub mod mcp;

use std::path::{Path, PathBuf};

use arch_facts::{ArchDir, Element, ElementId, SessionId, ToolLayerFile};

/// Errors of the tool layer.
#[derive(Debug, thiserror::Error)]
pub enum ToolLayerError {
    /// An `.arch/` file or the tool-layer state failed.
    #[error(transparent)]
    Facts(#[from] arch_facts::Error),
    /// The session has no `plan.toml` in this worktree.
    #[error("session {session}: no plan.toml under {worktree}/.arch/sessions/")]
    NoPlan {
        /// The session.
        session: SessionId,
        /// The worktree.
        worktree: PathBuf,
    },
    /// The plan has no such element.
    #[error("session {session}: no element {element} in plan.toml")]
    NoElement {
        /// The session.
        session: SessionId,
        /// The element.
        element: ElementId,
    },
    /// Reading or writing failed.
    #[error("{path}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// Git answered with an error.
    #[error("git {args}: {stderr}")]
    Git {
        /// The arguments.
        args: String,
        /// What git said.
        stderr: String,
    },
    /// A tool was called wrongly; the message is for the agent.
    #[error("{0}")]
    Invalid(String),
}

/// What the tool layer needs from the rest of arch, implemented by `arch-api`.
pub trait Project {
    /// The findings of `arch check` on the worktree: the `schemas/check.schema.json` document.
    fn check(&self, worktree: &Path) -> Result<serde_json::Value, String>;

    /// The area a worktree-relative file belongs to, for a commit's scope; `None` when no area
    /// claims it.
    fn area_of(&self, worktree: &Path, file: &Path) -> Option<String>;
}

/// One element of one session, in its worktree: what every hook call and MCP tool works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The worktree, canonical.
    pub worktree: PathBuf,
    /// The session.
    pub session: SessionId,
    /// The element, from the session's `plan.toml`.
    pub element: Element,
}

impl Scope {
    /// Load the element from `<worktree>/.arch/sessions/<session>/plan.toml`.
    pub fn load(
        worktree: &Path,
        session: &SessionId,
        element: &ElementId,
    ) -> Result<Self, ToolLayerError> {
        let worktree = worktree
            .canonicalize()
            .map_err(|source| ToolLayerError::Io {
                path: worktree.to_path_buf(),
                source,
            })?;
        let plan = ArchDir::of_repo(&worktree)
            .session(session)
            .read_plan()?
            .ok_or_else(|| ToolLayerError::NoPlan {
                session: session.clone(),
                worktree: worktree.clone(),
            })?;
        let element = plan
            .element(element)
            .cloned()
            .ok_or_else(|| ToolLayerError::NoElement {
                session: session.clone(),
                element: element.clone(),
            })?;
        Ok(Scope {
            worktree,
            session: session.clone(),
            element,
        })
    }

    /// The element's tool-layer state file.
    pub fn state_file(&self) -> ToolLayerFile {
        ToolLayerFile::new(
            &self.worktree,
            self.session.clone(),
            self.element.id.clone(),
        )
    }

    /// `raw` (absolute, or relative to the worktree) with symlinks resolved as far as the file
    /// system has it, so a path through a symlink is judged where it lands.
    pub fn locate(&self, raw: &Path) -> PathBuf {
        let joined = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            self.worktree.join(raw)
        };
        let joined = guard::normalize(&joined);
        let mut base = joined.as_path();
        let mut rest = vec![];
        loop {
            if let Ok(canonical) = base.canonicalize() {
                return rest.iter().rev().fold(canonical, |p, c| p.join(c));
            }
            match (base.parent(), base.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_os_string());
                    base = parent;
                }
                _ => return joined,
            }
        }
    }

    /// The file at a worktree-relative path, `None` when absent.
    pub fn on_disk(&self, rel: &Path) -> Option<Vec<u8>> {
        std::fs::read(self.worktree.join(rel)).ok()
    }
}
