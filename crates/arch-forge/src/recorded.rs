//! The test double: answers from recorded forge JSON and logs what it was sent (#45).
//!
//! Each recorded exchange answers once, the first unanswered one with the same method and path,
//! so a test can record the same request twice for "before" and "after". A request with no
//! recorded answer is a [`ForgeError::Transport`] naming it. Loaded from a directory, the double
//! is the fake forge behind `ARCH_FORGE=fake:<dir>` (#48).

use std::path::Path;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

use crate::{ForgeError, HttpRequest, HttpResponse, Method, Transport};

#[derive(Debug, Clone)]
struct Exchange {
    method: Method,
    path: String,
    status: u16,
    body: Value,
}

/// A transport that answers from recordings.
#[derive(Debug, Default)]
pub struct Recorded {
    exchanges: Mutex<Vec<Exchange>>,
    sent: Mutex<Vec<HttpRequest>>,
}

/// One entry of `recorded.json`: the body inline (`json`) or in a file next to it (`file`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    method: Method,
    path: String,
    status: u16,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    json: Option<Value>,
}

impl Recorded {
    /// A double with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an answer to `method path`.
    pub fn on(self, method: Method, path: &str, status: u16, body: Value) -> Self {
        self.exchanges.lock().unwrap().push(Exchange {
            method,
            path: path.into(),
            status,
            body,
        });
        self
    }

    /// The double recorded in `dir/recorded.json`: a list of
    /// `{"method", "path", "status", "file" | "json"}`, `file` relative to `dir`.
    pub fn from_dir(dir: &Path) -> Result<Self, ForgeError> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name)).map_err(|e| {
                ForgeError::Transport(format!("recorded: {}: {e}", dir.join(name).display()))
            })
        };
        let parse = |name: &str, text: &str| -> Result<Value, ForgeError> {
            serde_json::from_str(text)
                .map_err(|e| ForgeError::Decode(format!("recorded: {name}: {e}")))
        };
        let entries: Vec<Entry> =
            serde_json::from_value(parse("recorded.json", &read("recorded.json")?)?)
                .map_err(|e| ForgeError::Decode(format!("recorded: recorded.json: {e}")))?;
        let mut recorded = Recorded::new();
        for entry in entries {
            let body = match (entry.file, entry.json) {
                (Some(file), None) => parse(&file, &read(&file)?)?,
                (None, json) => json.unwrap_or(Value::Null),
                (Some(_), Some(_)) => {
                    return Err(ForgeError::Decode(format!(
                        "recorded: {} {}: both file and json",
                        entry.method, entry.path
                    )));
                }
            };
            recorded = recorded.on(entry.method, &entry.path, entry.status, body);
        }
        Ok(recorded)
    }

    /// The requests received so far, in order.
    pub fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl Transport for Recorded {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, ForgeError> {
        self.sent.lock().unwrap().push(request.clone());
        let mut exchanges = self.exchanges.lock().unwrap();
        let at = exchanges
            .iter()
            .position(|e| e.method == request.method && e.path == request.path)
            .ok_or_else(|| {
                ForgeError::Transport(format!(
                    "recorded: no answer for {} {}",
                    request.method, request.path
                ))
            })?;
        let exchange = exchanges.remove(at);
        Ok(HttpResponse {
            status: exchange.status,
            body: exchange.body,
        })
    }
}
