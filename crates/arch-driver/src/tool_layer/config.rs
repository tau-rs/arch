//! The files the claude-code driver is given ([`Context::settings`], [`Context::mcp_config`]):
//! arch's hooks and permissions, and arch's MCP server. Both run this same `arch` binary
//! ([`std::env::current_exe`]) on one element of one session.
//!
//! ```json
//! { "hooks": {
//!     "PreToolUse":         [ { "matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash", "hooks": [ { "type": "command", "command": "'/…/arch' hook pre --worktree '…' --session '…' --element '…'" } ] } ],
//!     "PostToolUse":        [ { "matcher": "Read|Edit|Write|MultiEdit|NotebookEdit", "hooks": [ … hook post … ] } ],
//!     "PostToolUseFailure": [ { "matcher": "Edit|Write|MultiEdit|NotebookEdit", "hooks": [ … hook post … ] } ] },
//!   "permissions": { "allow": [ "mcp__arch__read", …, "Bash(cargo test:*)", … ],
//!                    "deny": [ "Bash(git commit:*)", "Bash(git push:*)" ] } }
//!
//! { "mcpServers": { "arch": { "type": "stdio", "command": "/…/arch",
//!     "args": [ "mcp", "--worktree", "…", "--session", "…", "--element", "…", "--co-author", "…" ] } } }
//! ```
//!
//! The hooks are the confinement; the permissions only approve or refuse up front what the hook
//! would decide anyway, so a refused call costs no hook run.

use std::path::{Path, PathBuf};

use arch_facts::{ElementId, SessionId};
use serde_json::{Value, json};

use super::ToolLayerError;
use super::mcp::TOOLS;
use crate::Context;

/// Who the claude-code driver's commits are co-authored by (ADR 0016).
pub const CLAUDE_CO_AUTHOR: &str = "Claude <noreply@anthropic.com>";

/// Which `arch` runs the hooks and the MCP server, and on what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLayerArgs {
    /// The `arch` binary.
    pub exe: PathBuf,
    /// The session's worktree.
    pub worktree: PathBuf,
    /// The session.
    pub session: SessionId,
    /// The element.
    pub element: ElementId,
}

impl ToolLayerArgs {
    /// The running binary, on `element` of `session` in `worktree`.
    pub fn current(
        worktree: &Path,
        session: SessionId,
        element: ElementId,
    ) -> std::io::Result<Self> {
        Ok(ToolLayerArgs {
            exe: std::env::current_exe()?,
            worktree: worktree.to_path_buf(),
            session,
            element,
        })
    }

    fn args(&self, command: &[&str]) -> Vec<String> {
        let mut a: Vec<String> = command.iter().map(|s| s.to_string()).collect();
        a.extend([
            "--worktree".into(),
            self.worktree.display().to_string(),
            "--session".into(),
            self.session.to_string(),
            "--element".into(),
            self.element.to_string(),
        ]);
        a
    }

    fn command_line(&self, command: &[&str]) -> String {
        std::iter::once(self.exe.display().to_string())
            .chain(self.args(command))
            .map(|a| quote(&a))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The `--settings` document: arch's hooks and permissions.
pub fn settings(args: &ToolLayerArgs) -> Value {
    let hook = |phase: &str| json!([{ "type": "command", "command": args.command_line(&["hook", phase]) }]);
    let mut allow: Vec<String> = TOOLS.iter().map(|t| t.to_string()).collect();
    allow.extend(
        [
            "cargo check",
            "cargo build",
            "cargo test",
            "cargo clippy",
            "cargo fmt --check",
            "git status",
            "git diff",
            "git log",
            "git show",
            "git blame",
            "git ls-files",
            "git rev-parse",
        ]
        .iter()
        .map(|c| format!("Bash({c}:*)")),
    );
    json!({
        "hooks": {
            "PreToolUse": [{ "matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash", "hooks": hook("pre") }],
            "PostToolUse": [{ "matcher": "Read|Edit|Write|MultiEdit|NotebookEdit", "hooks": hook("post") }],
            "PostToolUseFailure": [{ "matcher": "Edit|Write|MultiEdit|NotebookEdit", "hooks": hook("post") }],
        },
        "permissions": {
            "allow": allow,
            "deny": ["Bash(git commit:*)", "Bash(git push:*)"],
        },
    })
}

/// The `--mcp-config` document: arch's server, named `arch`.
pub fn mcp_config(args: &ToolLayerArgs) -> Value {
    let mut a = args.args(&["mcp"]);
    a.extend(["--co-author".into(), CLAUDE_CO_AUTHOR.into()]);
    json!({
        "mcpServers": {
            "arch": { "type": "stdio", "command": args.exe.display().to_string(), "args": a }
        }
    })
}

/// Write both documents under `<worktree>/.arch/cache/driver/<element>/` (gitignored) and return
/// the [`Context`] naming them, with `pack` as the context pack.
pub fn write_context(
    args: &ToolLayerArgs,
    pack: Option<PathBuf>,
) -> Result<Context, ToolLayerError> {
    let dir = args
        .worktree
        .join(".arch/cache/driver")
        .join(args.element.as_str());
    std::fs::create_dir_all(&dir).map_err(|source| ToolLayerError::Io {
        path: dir.clone(),
        source,
    })?;
    let write = |name: &str, doc: Value| {
        let path = dir.join(name);
        let text = serde_json::to_string_pretty(&doc).expect("a JSON value serializes");
        std::fs::write(&path, text)
            .map(|()| path.clone())
            .map_err(|source| ToolLayerError::Io { path, source })
    };
    Ok(Context {
        pack,
        settings: Some(write("settings.json", settings(args))?),
        mcp_config: Some(write("mcp.json", mcp_config(args))?),
    })
}

/// `s` as one POSIX shell word.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_a_quote() {
        assert_eq!(quote("it's"), r"'it'\''s'");
    }
}
