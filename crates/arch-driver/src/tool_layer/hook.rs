//! `arch hook pre|post` around the [`guard`](super::guard): the hook JSON on stdin, the
//! element's state file, the Denial record, the exit code.
//!
//! Claude Code runs the command with the call on stdin (`hook_event_name`, `tool_name`,
//! `tool_input`, …). Exit 2 with a reason on stderr blocks the tool and the model reads the reason
//! (FINDINGS F-1). A pre hook that fails for any other cause must block too: the caller turns an
//! `Err` from [`run`] into exit 2 (fail closed). A post hook's failure blocks nothing.

use arch_facts::{ArchDir, ContentHash, RecordKind, Witness};
use serde::Deserialize;

use super::guard::{self, ToolCall, Verdict};
use super::{Scope, ToolLayerError};

/// Which hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `PreToolUse`: guard, veto, expect.
    Pre,
    /// `PostToolUse` / `PostToolUseFailure`: record the read, confirm or drop the write.
    Post,
}

/// What the hook process ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    /// 0 lets the call through; 2 blocks it.
    pub exit_code: u8,
    /// The reason, for the model.
    pub stderr: String,
}

impl HookOutcome {
    fn allow() -> Self {
        HookOutcome {
            exit_code: 0,
            stderr: String::new(),
        }
    }
}

#[derive(Deserialize)]
struct HookInput {
    #[serde(default)]
    hook_event_name: String,
    tool_name: String,
    #[serde(default)]
    tool_input: serde_json::Value,
}

/// Run one hook on the JSON Claude Code wrote to stdin.
pub fn run(phase: Phase, scope: &Scope, stdin: &str) -> Result<HookOutcome, ToolLayerError> {
    let input: HookInput = serde_json::from_str(stdin)
        .map_err(|e| ToolLayerError::Invalid(format!("arch hook: the hook input: {e}")))?;
    let call = ToolCall {
        tool: input.tool_name,
        input: input.tool_input,
    };
    let file = scope.state_file();
    match phase {
        Phase::Post => {
            let failed = input.hook_event_name == "PostToolUseFailure";
            file.update(|state| guard::post(&call, failed, scope, state, scope))?;
            Ok(HookOutcome::allow())
        }
        Phase::Pre => {
            let verdict = file.update(|state| {
                let verdict = guard::pre(&call, scope, state, scope);
                if let Verdict::Allow {
                    expect: Some((path, hash)),
                } = &verdict
                {
                    state.expect(path, hash.clone());
                }
                verdict
            })?;
            let Verdict::Deny(denial) = verdict else {
                return Ok(HookOutcome::allow());
            };
            let mut stderr = denial.reason.clone();
            let record = RecordKind::Denial {
                element: scope.element.id.clone(),
                path: denial.path.unwrap_or_default(),
                reason: denial.reason,
                agent_reason: denial.agent_reason,
            };
            let witness = Witness::Tool {
                tool: format!("arch hook pre · {}", call.tool),
                output_sha256: ContentHash::of_str(stdin).0,
            };
            if let Err(e) = ArchDir::of_repo(&scope.worktree)
                .session(&scope.session)
                .write_record(record, vec![witness])
            {
                stderr.push_str(&format!(" (arch could not record the denial: {e})"));
            }
            stderr.push('\n');
            Ok(HookOutcome {
                exit_code: 2,
                stderr,
            })
        }
    }
}
