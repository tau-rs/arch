//! Records (ADR 0003): gate outputs, judge verdicts, denials, overrides, resolution records.
//! Written as files under `.arch/sessions/<id>/records/`, each with its witnesses.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{ElementId, Timestamp};
use crate::model::Witness;

/// A judge's verdict on one element (ADR 0013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Realized as intended.
    Pass,
    /// Not realized, with a reason.
    Fail,
}

/// How a conflict was resolved (ADR 0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedBy {
    /// A resolve element run by an agent.
    Agent,
    /// By hand.
    Hand,
}

/// The record kinds (ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RecordKind {
    /// A gate command's output.
    GateOutput {
        /// The group whose gate ran.
        group: String,
        /// The command, or `arch check`.
        command: String,
        /// Exit code.
        exit_code: i32,
        /// Output.
        output: String,
    },
    /// A judge verdict (ADR 0013).
    JudgeVerdict {
        /// The element judged.
        element: ElementId,
        /// Verdict.
        verdict: Verdict,
        /// Reason.
        reason: String,
    },
    /// A write denied by the tool layer (ADR 0012).
    Denial {
        /// The element whose sub-agent wrote.
        element: ElementId,
        /// The path refused.
        path: PathBuf,
        /// arch's reason (stale write, outside scope).
        reason: String,
        /// The agent's stated reason, if it gave one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_reason: Option<String>,
    },
    /// A person overrode a gate or a verdict with a recorded reason (spec §6, ADR 0013).
    Override {
        /// What was overridden.
        what: String,
        /// Reason.
        reason: String,
        /// Who.
        by: String,
    },
    /// A resolution record (ADR 0019).
    Resolution {
        /// The resolve element.
        element: ElementId,
        /// Agent or hand.
        by: ResolvedBy,
        /// Summary.
        summary: String,
    },
}

impl RecordKind {
    /// The kind's name, used in file names.
    pub fn name(&self) -> &'static str {
        match self {
            RecordKind::GateOutput { .. } => "gate-output",
            RecordKind::JudgeVerdict { .. } => "judge-verdict",
            RecordKind::Denial { .. } => "denial",
            RecordKind::Override { .. } => "override",
            RecordKind::Resolution { .. } => "resolution",
        }
    }
}

/// A record file: sequence, time, kind, witnesses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Sequence number within the session, from 1.
    pub seq: u32,
    /// When.
    pub at: Timestamp,
    /// What.
    #[serde(flatten)]
    pub kind: RecordKind,
    /// Witnesses.
    #[serde(default)]
    pub witnesses: Vec<Witness>,
}
