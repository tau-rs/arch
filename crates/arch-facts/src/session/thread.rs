//! The session thread (ADR 0003): arch's own, the filtered driver stream plus pointers.
//! One [`ThreadEntry`] per line of `thread.jsonl`.

use serde::{Deserialize, Serialize};

use super::{DriverPointer, ElementId, Timestamp};

/// Who wrote a thread entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "kebab-case")]
pub enum ThreadAuthor {
    /// The planner.
    Planner,
    /// A sub-agent, on an element.
    Agent {
        /// The element it works on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element: Option<ElementId>,
    },
    /// The judge (ADR 0013).
    Judge,
    /// You.
    You,
    /// arch itself (What's new, restarted, denials).
    Arch,
}

/// What a thread entry says (the `TurnEvent` shape of `handoff-arch.md` §2, filtered, plus
/// arch's own lines from spec §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ThreadEvent {
    /// Text.
    Text {
        /// The text.
        text: String,
    },
    /// A tool call.
    ToolCall {
        /// Tool name.
        tool: String,
        /// A one-line summary of the input.
        summary: String,
    },
    /// A tool result.
    ToolResult {
        /// Tool name.
        tool: String,
        /// Whether it succeeded.
        ok: bool,
        /// A short excerpt.
        summary: String,
    },
    /// A sub-agent started on an element.
    Subagent {
        /// The element.
        element: ElementId,
    },
    /// A turn's result.
    Result {
        /// The summary.
        summary: String,
    },
    /// A question with options (spec §6 Asks); the turn ends.
    Ask {
        /// The questions.
        questions: Vec<Question>,
    },
    /// An answer to an ask.
    Answer {
        /// The answers, one per question, in order.
        answers: Vec<String>,
    },
    /// The planner's `changed · what` or `no change` line (spec §6).
    Changed {
        /// What changed; `None` is `no change`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<String>,
    },
    /// What's new told at a turn boundary (spec §6).
    WhatsNew {
        /// The lines.
        lines: Vec<String>,
    },
    /// A hand-back after take over (spec §6).
    HandBack {
        /// The element handed back.
        element: ElementId,
        /// The changed line.
        changed: String,
    },
    /// Resumed after a restart (ADR 0015).
    Restarted,
}

/// A batched question with options that say what each changes (spec §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// The question.
    pub text: String,
    /// Options.
    #[serde(default)]
    pub options: Vec<String>,
}

/// One line of `thread.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadEntry {
    /// When.
    pub at: Timestamp,
    /// Who.
    pub author: ThreadAuthor,
    /// What.
    #[serde(flatten)]
    pub event: ThreadEvent,
    /// The driver's pointers (ADR 0003), on the entries that come from a driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<DriverPointer>,
}

impl ThreadEntry {
    /// An entry now, by this author, with this event.
    pub fn new(author: ThreadAuthor, event: ThreadEvent) -> Self {
        ThreadEntry {
            at: super::now(),
            author,
            event,
            driver: None,
        }
    }
}
