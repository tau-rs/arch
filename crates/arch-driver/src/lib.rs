//! `arch-driver` · the `Driver` trait, the claude-code adapter, hooks and MCP tools (ADR 0012).
//!
//! The adapter, the stream-json parser and the `arch hook pre|post` behaviour land with
//! milestone 5; milestone 1 declares the boundary only.

/// Errors crossing the driver boundary.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The driver process failed to start or exited abnormally.
    #[error("driver: {0}")]
    Failed(String),
}

/// One event in a driver's turn stream (`handoff-arch.md` §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// Assistant text.
    Text(String),
    /// A tool call by name.
    ToolCall(String),
    /// A tool result by tool name.
    ToolResult(String),
    /// A sub-agent started.
    Subagent(String),
    /// The turn's final result.
    Result(String),
}

/// The agent runtime behind arch (spec §9): Claude Code first.
pub trait Driver {
    /// The driver's name, for the session record and the UI.
    fn name(&self) -> &'static str;
}
