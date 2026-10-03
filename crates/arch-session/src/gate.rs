//! The gate runner (spec §6): a group's commands, then `arch check`, each a `GateOutput` record
//! with its output as the witness. The judge ([`crate::judge`]) runs after them.
//!
//! ```text
//! sh -c "cargo test --workspace"  ─▶ 0001-gate-output.toml   exit 0
//! arch check (through the port)   ─▶ 0002-gate-output.toml   exit 0 · 0 blocking
//! ```

use std::path::Path;
use std::process::Command;

use arch_facts::{ContentHash, Group, RecordKind, SessionDir, Witness};

use crate::{Error, Project};

/// The command name `arch check`'s record carries.
pub const CHECK: &str = "arch check";

/// Output kept in a record, from the end: a failing test prints its failure last.
const KEEP: usize = 64 * 1024;

/// One command's run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRun {
    /// The command, or [`CHECK`].
    pub command: String,
    /// Exit code (`arch check`: 0 nothing blocks, 1 something blocks, 2 the check failed).
    pub exit_code: i32,
    /// What it printed, its tail when long.
    pub output: String,
}

impl CommandRun {
    /// Whether it passed.
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }
}

/// Run the group's commands and `arch check` in `worktree`, writing one record each.
pub fn run(
    group: &Group,
    worktree: &Path,
    dir: &SessionDir,
    project: &dyn Project,
) -> Result<Vec<CommandRun>, Error> {
    let mut runs = vec![];
    for command in &group.gate.commands {
        let out = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(worktree)
            .output()
            .map_err(|source| Error::Io {
                path: worktree.to_path_buf(),
                source,
            })?;
        let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
        output.push_str(&String::from_utf8_lossy(&out.stderr));
        runs.push(CommandRun {
            command: command.clone(),
            exit_code: out.status.code().unwrap_or(-1),
            output,
        });
    }
    if group.gate.check {
        runs.push(match project.check(worktree) {
            Ok(doc) => {
                let blocking = doc["summary"]["blocking"].as_u64().unwrap_or(0);
                CommandRun {
                    command: CHECK.into(),
                    exit_code: i32::from(blocking > 0),
                    output: serde_json::to_string_pretty(&doc).unwrap_or_default(),
                }
            }
            Err(reason) => CommandRun {
                command: CHECK.into(),
                exit_code: 2,
                output: reason,
            },
        });
    }
    for run in &mut runs {
        run.output = tail(&run.output, KEEP);
        dir.write_record(
            RecordKind::GateOutput {
                group: group.name.clone(),
                command: run.command.clone(),
                exit_code: run.exit_code,
                output: run.output.clone(),
            },
            vec![Witness::Tool {
                tool: run.command.clone(),
                output_sha256: ContentHash::of_str(&run.output).0,
            }],
        )?;
    }
    Ok(runs)
}

/// The last `max` bytes of `s`, cut at a character boundary.
pub(crate) fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

/// One line per run: `cargo test ✓ · arch check ✗ (exit 1)`.
pub fn summary(runs: &[CommandRun]) -> String {
    runs.iter()
        .map(|r| {
            if r.passed() {
                format!("{} ✓", r.command)
            } else {
                format!("{} ✗ (exit {})", r.command, r.exit_code)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
