//! `arch mcp`: the MCP server arch hands an agent, over stdio.
//!
//! A minimal JSON-RPC 2.0 server, one message per line: `initialize`, `ping`, `tools/list`,
//! `tools/call`; notifications get no answer. Four tools, for one element of one session
//! ([`Scope`]):
//!
//! | tool | input | does |
//! |---|---|---|
//! | `read` | `path` | the content and its sha256; records the hash for the stale-write guard |
//! | `check` | — | `arch check` on the worktree, through the [`Project`] port |
//! | `commit` | `type`, `summary`, `body?` | the element's files only, one commit per element ([`commit`](super::commit)) |
//! | `ask` | `questions[{text, options}]` | an `Ask` entry in the session thread; returns `wait` |
//!
//! Claude Code names them `mcp__arch__read`, … (the server is `arch` in the MCP config).

use std::io::{BufRead, Write};
use std::path::Path;

use arch_facts::{ArchDir, ContentHash, Question, ThreadAuthor, ThreadEntry, ThreadEvent};
use serde_json::{Value, json};

use super::commit::{self, CommitRequest};
use super::{Project, Scope, ToolLayerError};

/// The MCP protocol versions this server speaks; the first is offered when the client asks
/// for another.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The tool names, as an agent's allowed tools (`--allowedTools`).
pub const TOOLS: &[&str] = &[
    "mcp__arch__read",
    "mcp__arch__check",
    "mcp__arch__commit",
    "mcp__arch__ask",
];

/// The server for one element.
pub struct McpServer<'a> {
    scope: Scope,
    project: &'a dyn Project,
    co_author: String,
}

impl<'a> McpServer<'a> {
    /// A server over `scope`; commits are co-authored by `co_author` (`Name <email>`).
    pub fn new(scope: Scope, project: &'a dyn Project, co_author: impl Into<String>) -> Self {
        McpServer {
            scope,
            project,
            co_author: co_author.into(),
        }
    }

    /// Serve until `input` ends.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Answer one line; `None` for a notification.
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return id.map(|id| error(id, -32600, "invalid request: no method"));
        };
        let id = id?; // a notification
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&PROTOCOL_VERSIONS[0]);
                ok(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "arch", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": format!(
                            "arch's tools for element {} ({}) of session {}. Commit only with \
                             `commit`; ask the person with `ask`, then end your turn.",
                            self.scope.element.label, self.scope.element.id, self.scope.session
                        ),
                    }),
                )
            }
            "ping" => ok(id, json!({})),
            "tools/list" => ok(id, json!({ "tools": tool_list() })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let args = &params["arguments"];
                let result = match name {
                    "read" => self.read(args),
                    "check" => self.check(),
                    "commit" => self.commit(args),
                    "ask" => self.ask(args),
                    other => return Some(error(id, -32602, &format!("unknown tool: {other}"))),
                };
                ok(
                    id,
                    match result {
                        Ok(content) => json!({ "content": content, "isError": false }),
                        Err(e) => json!({ "content": [text(&e.to_string())], "isError": true }),
                    },
                )
            }
            other => error(id, -32601, &format!("method not found: {other}")),
        })
    }

    fn read(&self, args: &Value) -> Result<Vec<Value>, ToolLayerError> {
        let raw = args["path"]
            .as_str()
            .ok_or_else(|| invalid("read: `path` is required"))?;
        let at = self.scope.locate(Path::new(raw));
        let rel = at
            .strip_prefix(&self.scope.worktree)
            .map_err(|_| invalid(&format!("read: {raw} is outside the worktree")))?
            .to_path_buf();
        let bytes = std::fs::read(&at).map_err(|source| ToolLayerError::Io {
            path: rel.clone(),
            source,
        })?;
        let content = String::from_utf8(bytes)
            .map_err(|_| invalid(&format!("read: {} is not UTF-8 text", rel.display())))?;
        let hash = ContentHash::of_str(&content);
        self.scope
            .state_file()
            .update(|s| s.record_read(&rel, hash.clone()))?;
        Ok(vec![
            text(&format!("{} · sha256 {hash}", rel.display())),
            text(&content),
        ])
    }

    fn check(&self) -> Result<Vec<Value>, ToolLayerError> {
        let doc = self
            .project
            .check(&self.scope.worktree)
            .map_err(|e| ToolLayerError::Invalid(format!("arch check: {e}")))?;
        let pretty = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string());
        Ok(vec![text(&pretty)])
    }

    fn commit(&self, args: &Value) -> Result<Vec<Value>, ToolLayerError> {
        let request = CommitRequest {
            kind: args["type"]
                .as_str()
                .ok_or_else(|| invalid("commit: `type` is required"))?
                .to_string(),
            summary: args["summary"]
                .as_str()
                .ok_or_else(|| invalid("commit: `summary` is required"))?
                .to_string(),
            body: args["body"].as_str().map(str::to_string),
        };
        let area = self
            .scope
            .element
            .files
            .iter()
            .find_map(|f| self.project.area_of(&self.scope.worktree, f));
        let done = commit::commit(&self.scope, &request, area.as_deref(), &self.co_author)?;
        let files: Vec<String> = done.files.iter().map(|f| f.display().to_string()).collect();
        Ok(vec![text(&format!(
            "{} {} · {}\n{}",
            if done.amended { "amended" } else { "committed" },
            &done.hash[..done.hash.len().min(12)],
            files.join(", "),
            done.message.lines().next().unwrap_or_default()
        ))])
    }

    fn ask(&self, args: &Value) -> Result<Vec<Value>, ToolLayerError> {
        let questions: Vec<Question> =
            serde_json::from_value(args["questions"].clone()).map_err(|e| {
                invalid(&format!(
                    "ask: `questions` is a list of {{text, options}}: {e}"
                ))
            })?;
        if questions.is_empty() || questions.iter().any(|q| q.text.trim().is_empty()) {
            return Err(invalid("ask: at least one question, each with a text"));
        }
        let entry = ThreadEntry::new(
            ThreadAuthor::Agent {
                element: Some(self.scope.element.id.clone()),
            },
            ThreadEvent::Ask { questions },
        );
        ArchDir::of_repo(&self.scope.worktree)
            .session(&self.scope.session)
            .append_thread(&entry)?;
        Ok(vec![text("wait")])
    }
}

/// The four tools' descriptions and input schemas.
pub fn tool_list() -> Value {
    json!([
        {
            "name": "read",
            "description": "Read a file of the worktree: its content and sha256. arch refuses a \
                write to a file that changed since you last read it, so read before editing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative to the worktree, or absolute inside it." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "check",
            "description": "Run `arch check` on the worktree: the architecture findings (dependency \
                rules between areas), as the JSON document of schemas/check.schema.json.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "commit",
            "description": "Commit your element's files, and only them, as one Conventional Commit \
                `type(area): summary` with arch's trailers. Calling it again while that commit is \
                HEAD amends it. `git commit` through Bash is refused.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": commit::TYPES },
                    "summary": { "type": "string", "description": "One line, imperative." },
                    "body": { "type": "string", "description": "Defaults to the element's intention." }
                },
                "required": ["type", "summary"]
            }
        },
        {
            "name": "ask",
            "description": "Ask the person, in one batch, the questions only they can settle; give \
                each question options that say what each choice changes. Returns `wait`: end \
                your turn then, the answers come as the next message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" },
                                "options": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["text"]
                        }
                    }
                },
                "required": ["questions"]
            }
        }
    ])
}

fn text(s: &str) -> Value {
    json!({ "type": "text", "text": s })
}

fn invalid(message: &str) -> ToolLayerError {
    ToolLayerError::Invalid(message.to_string())
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
