//! `arch hook pre|post` and `arch mcp` (ADR 0012): the tool layer lives in `arch-driver`; this
//! module gives it the [`Project`] port (`check()` and a file's area need the analyzer and the
//! views) and is what the CLI calls.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use arch_driver::tool_layer::hook;
use arch_driver::tool_layer::mcp::McpServer;
use arch_driver::tool_layer::{Project, Scope};
use arch_facts::{ArchDir, ElementId, SessionId};
use arch_views::Placements;

pub use arch_driver::tool_layer::config::CLAUDE_CO_AUTHOR;
pub use arch_driver::tool_layer::hook::{HookOutcome, Phase};

use crate::Error;

/// Which element of which session, in which worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLayerTarget {
    /// The session's worktree.
    pub worktree: PathBuf,
    /// The session id.
    pub session: String,
    /// The element id.
    pub element: String,
}

impl ToolLayerTarget {
    fn scope(&self) -> Result<Scope, Error> {
        Ok(Scope::load(
            &self.worktree,
            &SessionId::new(&self.session),
            &ElementId::from_str_unchecked(&self.element),
        )?)
    }
}

/// Run one hook on the JSON Claude Code wrote to stdin. An `Err` from a pre hook must block the
/// call (exit 2): the tool layer fails closed.
pub fn hook(phase: Phase, target: &ToolLayerTarget, stdin: &str) -> Result<HookOutcome, Error> {
    Ok(hook::run(phase, &target.scope()?, stdin)?)
}

/// Serve the MCP tools for one element until `input` ends.
pub fn mcp(
    target: &ToolLayerTarget,
    co_author: &str,
    input: impl BufRead,
    output: impl Write,
) -> Result<(), Error> {
    let server = McpServer::new(target.scope()?, &ArchProject, co_author);
    server.serve(input, output).map_err(|source| Error::Io {
        path: PathBuf::from("<stdio>"),
        source,
    })
}

/// The [`Project`] port over arch-api's own methods.
struct ArchProject;

impl Project for ArchProject {
    fn check(&self, worktree: &Path) -> Result<serde_json::Value, String> {
        let output = crate::check(worktree).map_err(|e| e.to_string())?;
        serde_json::to_value(output).map_err(|e| e.to_string())
    }

    fn area_of(&self, worktree: &Path, file: &Path) -> Option<String> {
        let areas = ArchDir::of_repo(worktree).read_areas().ok()?;
        let placements = Placements::new(&areas).ok()?;
        placements
            .of_file(&file.to_string_lossy())
            .map(|p| p.area.clone())
    }
}
