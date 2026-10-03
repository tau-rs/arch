//! Sessions (spec §4, §6; ADR 0003, 0011, 0015).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::ElementId;

/// A session's id: the folder name under `.arch/sessions/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    /// Wrap an id.
    pub fn new(s: impl Into<String>) -> Self {
        SessionId(s.into())
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The session state machine's values (`handoff-arch.md` §2; spec §4, §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    /// Being planned; the plan is a draft in the cache (ADR 0020).
    Planning,
    /// A saved plan or a detected `you · <worktree>` session, locked (spec §4, §6).
    Yours,
    /// Sub-agents are working.
    Running,
    /// Waiting on an answer.
    Asks,
    /// A write outside the element's scope was denied; waiting on a typology.
    Deviation,
    /// A gate is running.
    Gate,
    /// The fix-round budget is spent; the four-door question is open.
    GateFailed,
    /// Handover written.
    Done,
    /// In review.
    InReview,
    /// Merged (forge fact, ADR 0003).
    Merged,
    /// Archived to `refs/notes/arch` (arch fact, ADR 0003).
    Archived,
}

/// Pointers into the driver's own records (ADR 0003): the thread is arch's own, these are
/// only pointers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverPointer {
    /// The driver's name.
    pub driver: String,
    /// The driver's session id.
    pub session_id: String,
    /// The driver's transcript path, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
}

/// A session (spec §7; ADR 0003, 0011, 0015): the record the scheduler resumes from,
/// `.arch/sessions/<id>/session.toml` on the session's branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Id.
    pub id: SessionId,
    /// Name shown in the rows.
    pub name: String,
    /// State.
    pub state: SessionState,
    /// Branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Worktree, `<parent>/<repo>-w<n>` (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    /// The base commit the branch started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The driver running it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<DriverPointer>,
    /// When it was created.
    pub created: super::Timestamp,
    /// Where the scheduler stands: what it resumes from (ADR 0015).
    #[serde(default)]
    pub cursor: Cursor,
    /// One driver session per element that has run; answers and fix rounds resume it
    /// (arch-design#33, option A).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentPointer>,
}

impl Session {
    /// The driver session an element ran in, if it has run.
    pub fn agent(&self, element: &ElementId) -> Option<&DriverPointer> {
        self.agents
            .iter()
            .find(|a| &a.element == element)
            .map(|a| &a.pointer)
    }

    /// Record the driver session an element runs in, replacing an earlier one.
    pub fn set_agent(&mut self, element: ElementId, pointer: DriverPointer) {
        match self.agents.iter_mut().find(|a| a.element == element) {
            Some(a) => a.pointer = pointer,
            None => self.agents.push(AgentPointer { element, pointer }),
        }
    }
}

/// The scheduler's position in the plan (spec §11: one group at a time, its elements one after
/// another), written after every step so a restart resumes at the last turn boundary (ADR 0015).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// The running group: its index in `plan.toml`'s groups.
    #[serde(default)]
    pub group: u32,
    /// The group's elements still to run in this pass (the first run or a fix round), in order;
    /// the first one is running.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo: Vec<ElementId>,
    /// Fix rounds the group's gate has used (budget: `Gate::fix_rounds`).
    #[serde(default)]
    pub fix_round: u8,
    /// Rounds granted past the budget by "one more round" (spec §6).
    #[serde(default)]
    pub extra_rounds: u8,
    /// What a person must answer before the scheduler moves on: an ask or a deviation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<Waiting>,
}

/// The element a session waits on (states `asks` and `deviation`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    /// The element whose agent asked, or whose write was denied.
    pub element: ElementId,
    /// The paths denied as outside the element (deviation only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied: Vec<PathBuf>,
}

/// The driver session an element runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPointer {
    /// The element.
    pub element: ElementId,
    /// Its driver session.
    #[serde(flatten)]
    pub pointer: DriverPointer,
}
