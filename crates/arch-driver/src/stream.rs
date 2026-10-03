//! The claude-code stream-json parser: one JSON object per line in, [`TurnEvent`]s out.
//!
//! Lenient by design: the CLI adds line types and fields between releases. A line that is not
//! JSON is skipped, a type arch does not read is counted as unknown, and neither fails the turn.
//! Shapes are pinned by the recordings under `tests/fixtures/` (`experiments/record-stream.sh`).

use serde_json::Value;

use crate::TurnEvent;

/// The last event of a turn: how it ended.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// The driver's session id.
    pub session_id: String,
    /// False when the turn ended in error (`is_error`).
    pub ok: bool,
    /// How it ended: `success`, `error_max_turns`, `error_during_execution`, …
    pub subtype: String,
    /// The final answer's text, on success.
    pub text: Option<String>,
    /// The structured answer when the task carried an output schema.
    pub structured: Option<Value>,
    /// Agentic turns used.
    pub num_turns: u32,
    /// Cost as reported by the CLI.
    pub cost_usd: Option<f64>,
    /// Tools whose calls were denied during the turn, by name, in order.
    pub denials: Vec<String>,
}

/// Line counts, for the session record and for noticing a CLI format drift.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Non-blank lines read.
    pub lines: u32,
    /// Lines that were not JSON objects.
    pub skipped: u32,
    /// JSON lines of a type the parser does not read.
    pub unknown: u32,
    /// Result lines.
    pub results: u32,
}

/// Turns stream-json lines into events, one line at a time.
#[derive(Debug, Default)]
pub struct StreamParser {
    stats: StreamStats,
}

impl StreamParser {
    /// The counts so far.
    pub fn stats(&self) -> StreamStats {
        self.stats
    }

    /// Parse one line; a line yields zero or more events (an assistant message holds blocks).
    pub fn line(&mut self, line: &str) -> Vec<TurnEvent> {
        let line = line.trim();
        if line.is_empty() {
            return vec![];
        }
        self.stats.lines += 1;
        let Ok(Value::Object(v)) = serde_json::from_str::<Value>(line) else {
            self.stats.skipped += 1;
            return vec![];
        };
        let v = Value::Object(v);
        match (str_at(&v, "type"), str_at(&v, "subtype")) {
            ("system", "init") => vec![TurnEvent::Started {
                session_id: str_at(&v, "session_id").to_string(),
                model: str_at(&v, "model").to_string(),
                tools: v["tools"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            }],
            ("system", "task_started") => vec![TurnEvent::Subagent {
                id: str_at(&v, "tool_use_id").to_string(),
                kind: str_at(&v, "subagent_type").to_string(),
                description: str_at(&v, "description").to_string(),
            }],
            ("system", "hook_response") => vec![TurnEvent::Hook {
                event: str_at(&v, "hook_event").to_string(),
                name: str_at(&v, "hook_name").to_string(),
                exit_code: v["exit_code"].as_i64().map(|c| c as i32),
                output: str_at(&v, "output").to_string(),
            }],
            // Progress and bookkeeping arch does not surface.
            ("system", _) | ("rate_limit_event", _) | ("stream_event", _) => vec![],
            ("assistant", _) => assistant(&v),
            ("user", _) => user(&v),
            ("result", _) => {
                self.stats.results += 1;
                vec![TurnEvent::Result(result(&v))]
            }
            _ => {
                self.stats.unknown += 1;
                vec![]
            }
        }
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn parent(v: &Value) -> Option<String> {
    v["parent_tool_use_id"].as_str().map(String::from)
}

fn blocks(v: &Value) -> &[Value] {
    v["message"]["content"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn assistant(v: &Value) -> Vec<TurnEvent> {
    let subagent = parent(v);
    blocks(v)
        .iter()
        .filter_map(|b| match str_at(b, "type") {
            "text" => Some(TurnEvent::Text {
                text: str_at(b, "text").to_string(),
                subagent: subagent.clone(),
            }),
            "tool_use" => Some(TurnEvent::ToolCall {
                id: str_at(b, "id").to_string(),
                name: str_at(b, "name").to_string(),
                input: b["input"].clone(),
                subagent: subagent.clone(),
            }),
            // Thinking and anything newer.
            _ => None,
        })
        .collect()
}

fn user(v: &Value) -> Vec<TurnEvent> {
    let subagent = parent(v);
    blocks(v)
        .iter()
        .filter(|b| str_at(b, "type") == "tool_result")
        .map(|b| TurnEvent::ToolResult {
            id: str_at(b, "tool_use_id").to_string(),
            ok: !b["is_error"].as_bool().unwrap_or(false),
            content: content_text(&b["content"]),
            subagent: subagent.clone(),
        })
        .collect()
}

/// A tool result's content is a string or a list of blocks.
fn content_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn result(v: &Value) -> TurnResult {
    let ok = !v["is_error"].as_bool().unwrap_or(true);
    TurnResult {
        session_id: str_at(v, "session_id").to_string(),
        ok,
        subtype: str_at(v, "subtype").to_string(),
        text: v["result"].as_str().filter(|_| ok).map(String::from),
        structured: Some(v["structured_output"].clone()).filter(|s| !s.is_null()),
        num_turns: v["num_turns"].as_u64().unwrap_or(0) as u32,
        cost_usd: v["total_cost_usd"].as_f64(),
        denials: v["permission_denials"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|d| str_at(d, "tool_name").to_string())
                    .collect()
            })
            .unwrap_or_default(),
    }
}
