//! The claude-code adapter: `claude -p` in the worktree, stream-json out (ADR 0012).
//!
//! Command line from FINDINGS F-1, amended by arch-design#33 option A:
//!
//! ```text
//! claude -p --output-format stream-json --verbose --include-hook-events
//!        --setting-sources '' --strict-mcp-config
//!        --session-id <uuid> | --resume <uuid>
//!        --settings <hooks> --mcp-config <arch> --append-system-prompt-file <pack>
//!        --tools <built-ins> --allowedTools <approved> [--json-schema …] [--model …]
//!        --permission-mode acceptEdits --permission-prompts none
//! ```
//!
//! The prompt goes in on stdin: `--tools` and `--allowedTools` take several values and would
//! swallow a trailing positional prompt.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use arch_facts::DriverPointer;

use crate::stream::StreamParser;
use crate::{Context, Driver, DriverError, Task, Turn, TurnEvent, TurnHandle};

/// How much of stderr is kept for an error message.
const STDERR_TAIL: usize = 4096;

/// The claude-code driver.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    program: OsString,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        ClaudeCode {
            program: "claude".into(),
        }
    }
}

impl ClaudeCode {
    /// The adapter over `claude` on the `PATH`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The adapter over another binary (a pinned install, a test stub).
    pub fn with_program(program: impl Into<OsString>) -> Self {
        ClaudeCode {
            program: program.into(),
        }
    }

    /// The argv after the program name, for a new session (`resume == false`) or a resume.
    pub fn args(
        &self,
        resume: bool,
        session_id: &str,
        task: &Task,
        context: &Context,
    ) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        let mut push = |parts: &[&OsStr]| a.extend(parts.iter().map(|p| p.to_os_string()));
        push(&[
            "-p".as_ref(),
            "--output-format".as_ref(),
            "stream-json".as_ref(),
            "--verbose".as_ref(),
            "--include-hook-events".as_ref(),
            "--setting-sources".as_ref(),
            "".as_ref(),
            "--strict-mcp-config".as_ref(),
        ]);
        let flag = if resume { "--resume" } else { "--session-id" };
        push(&[flag.as_ref(), session_id.as_ref()]);
        if let Some(model) = &task.model {
            push(&["--model".as_ref(), model.as_ref()]);
        }
        if let Some(p) = &context.settings {
            push(&["--settings".as_ref(), p.as_os_str()]);
        }
        if let Some(p) = &context.mcp_config {
            push(&["--mcp-config".as_ref(), p.as_os_str()]);
        }
        if let Some(p) = &context.pack {
            push(&["--append-system-prompt-file".as_ref(), p.as_os_str()]);
        }
        if !task.tools.is_empty() {
            push(&["--tools".as_ref(), task.tools.join(",").as_ref()]);
        }
        if !task.allowed_tools.is_empty() {
            push(&[
                "--allowedTools".as_ref(),
                task.allowed_tools.join(",").as_ref(),
            ]);
        }
        if let Some(schema) = &task.output_schema {
            push(&["--json-schema".as_ref(), schema.to_string().as_ref()]);
        }
        if let Some(n) = task.max_turns {
            push(&["--max-turns".as_ref(), n.to_string().as_ref()]);
        }
        push(&[
            "--permission-mode".as_ref(),
            "acceptEdits".as_ref(),
            "--permission-prompts".as_ref(),
            "none".as_ref(),
        ]);
        a
    }

    /// The ADR 0003 pointer for a session run in `cwd`.
    pub fn pointer(&self, cwd: &Path, session_id: &str) -> DriverPointer {
        DriverPointer {
            driver: self.name().to_string(),
            session_id: session_id.to_string(),
            transcript_path: transcript_path(cwd, session_id),
        }
    }

    fn run(
        &self,
        resume: bool,
        session_id: String,
        task: &Task,
        context: &Context,
    ) -> Result<Turn, DriverError> {
        let program = self.program.to_string_lossy().into_owned();
        let mut child = Command::new(&self.program)
            .args(self.args(resume, &session_id, task, context))
            .current_dir(&task.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| DriverError::Spawn {
                program: program.clone(),
                source,
            })?;
        // The CLI reads all of stdin before starting, so this write cannot block on it.
        let mut stdin = child.stdin.take().expect("piped");
        stdin.write_all(task.prompt.as_bytes())?;
        drop(stdin);
        let stdout = child.stdout.take().expect("piped");
        let mut err_pipe = child.stderr.take().expect("piped");
        let stderr = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            let cut = buf.len().saturating_sub(STDERR_TAIL);
            let cut = (cut..buf.len())
                .find(|&i| buf.is_char_boundary(i))
                .unwrap_or(buf.len());
            buf[cut..].to_string()
        });
        let child = Arc::new(Mutex::new(child));
        let events = ClaudeTurn {
            program,
            lines: BufReader::new(stdout),
            parser: StreamParser::default(),
            pending: VecDeque::new(),
            child: Arc::clone(&child),
            stderr: Some(stderr),
            saw_result: false,
            done: false,
        };
        Ok(Turn::new(
            session_id,
            Box::new(events),
            TurnHandle::of(child),
        ))
    }
}

impl Driver for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn start(&mut self, task: &Task, context: &Context) -> Result<Turn, DriverError> {
        self.run(false, uuid::Uuid::new_v4().to_string(), task, context)
    }

    fn resume(
        &mut self,
        session_id: &str,
        task: &Task,
        context: &Context,
    ) -> Result<Turn, DriverError> {
        self.run(true, session_id.to_string(), task, context)
    }
}

/// Where Claude Code keeps a session's transcript: `<config>/projects/<cwd>/<id>.jsonl`, the
/// directory named by the canonical cwd with every non-alphanumeric character as `-`. The config
/// dir is `$CLAUDE_CONFIG_DIR`, else `~/.claude`. `None` without a home directory.
pub fn transcript_path(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    let config = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".claude"),
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let name: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Some(
        config
            .join("projects")
            .join(name)
            .join(format!("{session_id}.jsonl")),
    )
}

struct ClaudeTurn {
    program: String,
    lines: BufReader<ChildStdout>,
    parser: StreamParser,
    pending: VecDeque<TurnEvent>,
    child: Arc<Mutex<std::process::Child>>,
    stderr: Option<std::thread::JoinHandle<String>>,
    saw_result: bool,
    done: bool,
}

impl Iterator for ClaudeTurn {
    type Item = Result<TurnEvent, DriverError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                self.saw_result |= matches!(e, TurnEvent::Result(_));
                return Some(Ok(e));
            }
            if self.done {
                return None;
            }
            let mut line = String::new();
            match self.lines.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    return self.exited().err().map(Err);
                }
                Ok(_) => self.pending.extend(self.parser.line(&line)),
                Err(e) => {
                    self.done = true;
                    return Some(Err(e.into()));
                }
            }
        }
    }
}

impl ClaudeTurn {
    /// Reap the process at end of stream; an exit without a result line is an error.
    fn exited(&mut self) -> Result<(), DriverError> {
        let status = self
            .child
            .lock()
            .map_err(|_| std::io::Error::other("turn handle lock poisoned"))?
            .wait()?;
        if self.saw_result {
            return Ok(());
        }
        let stderr = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        Err(DriverError::Exited {
            program: self.program.clone(),
            status: status.to_string(),
            stderr: stderr.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_dir_is_the_cwd_with_non_alphanumerics_as_dashes() {
        let dir = tempfile::tempdir().unwrap();
        let p = transcript_path(dir.path(), "abc").unwrap();
        let name = p
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with('-'));
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_eq!(p.file_name().unwrap(), "abc.jsonl");
        assert_eq!(
            p.parent().unwrap().parent().unwrap().file_name().unwrap(),
            "projects"
        );
    }
}
