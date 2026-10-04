//! The acting replay: what Claude Code does around each tool call, done for a recorded turn.
//!
//! A recorded turn carries the agent's tool calls but none of their effects. Acting plays each
//! top-level call the way Claude Code would, through the files arch handed the driver
//! ([`Context::settings`], [`Context::mcp_config`]):
//!
//! ```text
//! Write / Edit   PreToolUse hooks ─ exit 2 ─▶ denied, nothing written
//!                          └─ exit 0 ─▶ apply ─▶ PostToolUse (PostToolUseFailure) hooks
//! Read           read ─▶ PostToolUse hooks
//! mcp__<s>__<t>  tools/call on server <s>, started from the MCP config at its first call
//! anything else  nothing
//! ```
//!
//! The recorded tool results are still what the stream shows; only the effects are real. A path
//! in a tool input is made absolute against the task's cwd first, as Claude Code's are.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

use crate::{Context, DriverError, TurnEvent};

/// The tool inputs that name a file.
const PATH_KEYS: [&str; 2] = ["file_path", "notebook_path"];

/// Acts out one turn's tool calls; the MCP servers live until it is dropped.
pub(crate) struct Actor {
    cwd: PathBuf,
    session_id: String,
    hooks: Value,
    mcp: Value,
    servers: Vec<(String, Server)>,
    next_id: u64,
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Actor {
    pub(crate) fn new(
        cwd: &Path,
        session_id: &str,
        context: &Context,
    ) -> Result<Self, DriverError> {
        let read = |p: &Option<PathBuf>| -> Result<Value, DriverError> {
            match p {
                None => Ok(Value::Null),
                Some(p) => std::fs::read_to_string(p)
                    .map_err(|e| replay(format!("{}: {e}", p.display())))
                    .and_then(|t| {
                        serde_json::from_str(&t)
                            .map_err(|e| replay(format!("{}: {e}", p.display())))
                    }),
            }
        };
        Ok(Actor {
            cwd: cwd.to_path_buf(),
            session_id: session_id.to_string(),
            hooks: read(&context.settings)?["hooks"].clone(),
            mcp: read(&context.mcp_config)?["mcpServers"].clone(),
            servers: vec![],
            next_id: 1,
        })
    }

    /// Do what Claude Code does for `event`, when it is a top-level tool call.
    pub(crate) fn act(&mut self, event: &TurnEvent) -> Result<(), DriverError> {
        let TurnEvent::ToolCall {
            name,
            input,
            subagent: None,
            ..
        } = event
        else {
            return Ok(());
        };
        let input = self.absolute(input);
        match name.as_str() {
            "Write" | "Edit" => {
                if self.hooks_say_no("PreToolUse", name, &input)? {
                    return Ok(());
                }
                let applied = apply(name, &input);
                let event = match applied {
                    Ok(()) => "PostToolUse",
                    Err(_) => "PostToolUseFailure",
                };
                self.hooks_say_no(event, name, &input)?;
                Ok(())
            }
            "Read" => {
                self.hooks_say_no("PostToolUse", name, &input)?;
                Ok(())
            }
            mcp if mcp.starts_with("mcp__") => {
                let mut parts = mcp["mcp__".len()..].splitn(2, "__");
                let (Some(server), Some(tool)) = (parts.next(), parts.next()) else {
                    return Ok(());
                };
                self.call(server, tool, &input)
            }
            _ => Ok(()),
        }
    }

    fn absolute(&self, input: &Value) -> Value {
        let mut input = input.clone();
        for key in PATH_KEYS {
            if let Some(p) = input[key].as_str()
                && Path::new(p).is_relative()
            {
                input[key] = json!(self.cwd.join(p).display().to_string());
            }
        }
        input
    }

    /// Run the hooks registered for `event` on `tool`; true when one blocked (exit 2).
    fn hooks_say_no(&self, event: &str, tool: &str, input: &Value) -> Result<bool, DriverError> {
        let stdin = json!({
            "session_id": self.session_id,
            "cwd": self.cwd.display().to_string(),
            "hook_event_name": event,
            "tool_name": tool,
            "tool_input": input,
        })
        .to_string();
        let groups = self.hooks[event].as_array().cloned().unwrap_or_default();
        for group in groups {
            let matcher = group["matcher"].as_str().unwrap_or_default();
            if !matcher.is_empty() && !matcher.split('|').any(|m| m == tool) {
                continue;
            }
            for hook in group["hooks"].as_array().into_iter().flatten() {
                let Some(command) = hook["command"].as_str() else {
                    continue;
                };
                let mut child = Command::new("sh")
                    .args(["-c", command])
                    .current_dir(&self.cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| replay(format!("hook {command}: {e}")))?;
                child
                    .stdin
                    .take()
                    .expect("piped")
                    .write_all(stdin.as_bytes())?;
                if child.wait()?.code() == Some(2) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// `tools/call` on `server`, started (and initialized) at its first call.
    fn call(&mut self, server: &str, tool: &str, input: &Value) -> Result<(), DriverError> {
        if !self.servers.iter().any(|(n, _)| n == server) {
            let started = self.start(server)?;
            self.servers.push((server.to_string(), started));
            self.request(
                server,
                "initialize",
                json!({ "protocolVersion": "2025-06-18" }),
            )?;
        }
        self.request(
            server,
            "tools/call",
            json!({ "name": tool, "arguments": input }),
        )?;
        Ok(())
    }

    fn start(&self, server: &str) -> Result<Server, DriverError> {
        let spec = &self.mcp[server];
        let command = spec["command"]
            .as_str()
            .ok_or_else(|| replay(format!("no MCP server {server} in the MCP config")))?;
        let args: Vec<&str> = spec["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut child = Command::new(command)
            .args(&args)
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| replay(format!("MCP server {server}: {e}")))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Server {
            child,
            stdin,
            stdout,
        })
    }

    fn request(&mut self, server: &str, method: &str, params: Value) -> Result<Value, DriverError> {
        let id = self.next_id;
        self.next_id += 1;
        let s = self
            .servers
            .iter_mut()
            .find(|(n, _)| n == server)
            .map(|(_, s)| s)
            .expect("started");
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(s.stdin, "{line}")?;
        s.stdin.flush()?;
        let mut reply = String::new();
        if s.stdout.read_line(&mut reply)? == 0 {
            return Err(replay(format!("MCP server {server} closed on {method}")));
        }
        serde_json::from_str(&reply).map_err(|e| replay(format!("MCP server {server}: {e}")))
    }
}

/// Apply a `Write` or an `Edit` to the disk.
fn apply(tool: &str, input: &Value) -> std::io::Result<()> {
    let path = Path::new(input["file_path"].as_str().unwrap_or_default());
    match tool {
        "Write" => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, input["content"].as_str().unwrap_or_default())
        }
        _ => {
            let old = input["old_string"].as_str().unwrap_or_default();
            let new = input["new_string"].as_str().unwrap_or_default();
            let text = std::fs::read_to_string(path)?;
            if old.is_empty() || !text.contains(old) {
                return Err(std::io::Error::other("old_string not found"));
            }
            let text = if input["replace_all"].as_bool() == Some(true) {
                text.replace(old, new)
            } else {
                text.replacen(old, new, 1)
            };
            std::fs::write(path, text)
        }
    }
}

fn replay(message: String) -> DriverError {
    DriverError::Replay(message)
}

/// A turn's events with each tool call acted out after it is handed on and before the next
/// event, as Claude Code runs a tool between the call and what follows it.
pub(crate) struct Acting<I> {
    events: I,
    actor: Option<Actor>,
    pending: Option<TurnEvent>,
}

impl<I> Acting<I> {
    pub(crate) fn new(events: I, actor: Actor) -> Self {
        Acting {
            events,
            actor: Some(actor),
            pending: None,
        }
    }
}

impl<I: Iterator<Item = Result<TurnEvent, DriverError>>> Iterator for Acting<I> {
    type Item = Result<TurnEvent, DriverError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let (Some(call), Some(actor)) = (self.pending.take(), self.actor.as_mut())
            && let Err(e) = actor.act(&call)
        {
            return Some(Err(e));
        }
        let item = self.events.next();
        match &item {
            Some(Ok(event @ TurnEvent::ToolCall { .. })) => self.pending = Some(event.clone()),
            None => self.actor = None,
            _ => {}
        }
        item
    }
}
