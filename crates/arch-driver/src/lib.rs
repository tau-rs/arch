//! `arch-driver` · the `Driver` trait, the claude-code adapter, hooks and MCP tools (ADR 0012).
//!
//! A driver runs an agent CLI in a session's worktree and streams its turn back as
//! [`TurnEvent`]s. The claude-code adapter ([`ClaudeCode`]) spawns `claude -p` with the command
//! line of FINDINGS F-1 (no `--bare`) and arch-design#33 option A (`--session-id` / `--resume`,
//! no `--no-session-persistence`). The [`ReplayDriver`] plays recorded stream-json instead, so the
//! session engine is testable without a Claude login. The [`tool_layer`] is the agent's
//! confinement: `arch hook pre|post`, the MCP tools, and the files that hand them to the driver.

mod acting;
mod claude_code;
mod replay;
mod stream;
pub mod tool_layer;

use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};

pub use claude_code::{ClaudeCode, transcript_path};
pub use replay::{ReplayCall, ReplayDriver};
pub use stream::{StreamParser, StreamStats, TurnResult};

/// Errors crossing the driver boundary.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The agent CLI could not be started.
    #[error("driver: cannot start {program}: {source}")]
    Spawn {
        /// The program spawned.
        program: String,
        /// Why it failed.
        #[source]
        source: std::io::Error,
    },
    /// Reading the agent's output failed.
    #[error("driver: reading the turn: {0}")]
    Io(#[from] std::io::Error),
    /// The agent exited without a result line.
    #[error("driver: {program} exited ({status}) without a result: {stderr}")]
    Exited {
        /// The program.
        program: String,
        /// Its exit status.
        status: String,
        /// The tail of what it wrote on stderr.
        stderr: String,
    },
    /// The turn's events ended without a result line.
    #[error("driver: the turn ended without a result")]
    NoResult,
    /// A recorded turn could not be replayed.
    #[error("driver: replay: {0}")]
    Replay(String),
}

/// What the agent is asked to do in one turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Task {
    /// The prompt (for a resume, the message).
    pub prompt: String,
    /// The working directory: the session's worktree.
    pub cwd: PathBuf,
    /// The built-in tools the agent has at all (`--tools`); empty means the CLI's default set.
    pub tools: Vec<String>,
    /// Tools approved without a prompt (`--allowedTools`), e.g. `mcp__arch__commit`.
    /// Approving is not restricting: [`Task::tools`] is the restriction.
    pub allowed_tools: Vec<String>,
    /// A JSON schema the final answer must match (`--json-schema`): planner and judge.
    pub output_schema: Option<serde_json::Value>,
    /// The model, when not the CLI's default.
    pub model: Option<String>,
    /// A cap on agentic turns.
    pub max_turns: Option<u32>,
}

/// What surrounds every turn of a session: the context pack and the confinement (ADR 0005,
/// ADR 0012). Paths to files arch wrote; the driver only passes them on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The context pack, appended to the system prompt.
    pub pack: Option<PathBuf>,
    /// Settings carrying arch's hooks and permissions (#46).
    pub settings: Option<PathBuf>,
    /// The MCP config naming arch's server (#46).
    pub mcp_config: Option<PathBuf>,
}

/// One event of a turn (`handoff-arch.md` §2), in stream order.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    /// The agent started: its session id, model and available tools.
    Started {
        /// The driver's session id.
        session_id: String,
        /// The model.
        model: String,
        /// The tools the agent has.
        tools: Vec<String>,
    },
    /// Assistant text.
    Text {
        /// The text.
        text: String,
        /// The sub-agent's tool-call id when a sub-agent wrote it.
        subagent: Option<String>,
    },
    /// A tool call.
    ToolCall {
        /// The call's id.
        id: String,
        /// Tool name (`Edit`, `mcp__arch__commit`).
        name: String,
        /// Tool input.
        input: serde_json::Value,
        /// The sub-agent's tool-call id when a sub-agent made the call.
        subagent: Option<String>,
    },
    /// A tool result.
    ToolResult {
        /// The id of the call it answers.
        id: String,
        /// False when the tool failed or was denied (a hook's exit 2 lands here).
        ok: bool,
        /// The result text.
        content: String,
        /// The sub-agent's tool-call id when it answers a sub-agent's call.
        subagent: Option<String>,
    },
    /// A sub-agent started.
    Subagent {
        /// The tool-call id that spawned it; later events carry it as `subagent`.
        id: String,
        /// Its kind (`general-purpose`).
        kind: String,
        /// What it was asked.
        description: String,
    },
    /// A hook ran (needs `--include-hook-events`).
    Hook {
        /// The hook event (`PreToolUse`).
        event: String,
        /// The hook's name (`PreToolUse:Edit`).
        name: String,
        /// Its exit code; 2 blocks the tool.
        exit_code: Option<i32>,
        /// What it wrote.
        output: String,
    },
    /// The turn's result; the last event.
    Result(TurnResult),
}

/// The events of one turn, in order. Iterating blocks on the agent.
pub struct Turn {
    session_id: String,
    events: Box<dyn Iterator<Item = Result<TurnEvent, DriverError>> + Send>,
    handle: TurnHandle,
}

impl Turn {
    pub(crate) fn new(
        session_id: String,
        events: Box<dyn Iterator<Item = Result<TurnEvent, DriverError>> + Send>,
        handle: TurnHandle,
    ) -> Self {
        Turn {
            session_id,
            events,
            handle,
        }
    }

    /// The driver's session id, known before the first event (arch chose it).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// A handle to interrupt or stop the turn from another thread.
    pub fn handle(&self) -> TurnHandle {
        self.handle.clone()
    }

    /// Drain the turn and return its result.
    pub fn finish(self) -> Result<TurnResult, DriverError> {
        let mut result = None;
        for event in self {
            if let TurnEvent::Result(r) = event? {
                result = Some(r);
            }
        }
        result.ok_or(DriverError::NoResult)
    }
}

impl Iterator for Turn {
    type Item = Result<TurnEvent, DriverError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.events.next()
    }
}

/// Interrupts or stops a running turn. A replayed turn's handle does nothing.
#[derive(Clone, Default)]
pub struct TurnHandle {
    child: Option<Arc<Mutex<Child>>>,
}

impl TurnHandle {
    pub(crate) fn of(child: Arc<Mutex<Child>>) -> Self {
        TurnHandle { child: Some(child) }
    }

    /// Ask the agent to stop at the next safe point (SIGINT); the turn still ends with its
    /// remaining events.
    pub fn interrupt(&self) -> Result<(), DriverError> {
        let Some(child) = &self.child else {
            return Ok(());
        };
        let pid = child.lock().map_err(poisoned)?.id();
        std::process::Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .status()?;
        Ok(())
    }

    /// Kill the agent now (SIGKILL).
    pub fn stop(&self) -> Result<(), DriverError> {
        let Some(child) = &self.child else {
            return Ok(());
        };
        let mut child = child.lock().map_err(poisoned)?;
        match child.kill() {
            Ok(()) => Ok(()),
            // Already exited.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> DriverError {
    DriverError::Io(std::io::Error::other("turn handle lock poisoned"))
}

/// The agent runtime behind arch (spec §9): Claude Code first.
///
/// Each turn is its own process: `start` opens a driver session, `resume` continues it with a
/// message (an answer to an ask, a fix round, a restart; arch-design#33). Both take the task and
/// context again because nothing survives between processes but the driver's session. Interrupt
/// and stop are on the [`TurnHandle`], so a turn can be stopped from another thread.
pub trait Driver {
    /// The driver's name, for the session record and the UI.
    fn name(&self) -> &'static str;

    /// Start a new driver session with a first turn.
    fn start(&mut self, task: &Task, context: &Context) -> Result<Turn, DriverError>;

    /// Continue the driver session `session_id`; `task.prompt` is the message.
    fn resume(
        &mut self,
        session_id: &str,
        task: &Task,
        context: &Context,
    ) -> Result<Turn, DriverError>;
}
