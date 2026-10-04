//! The HTTP transport port, and its production adapter over `ureq`.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ForgeError, Token};

/// The HTTP methods the adapters use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        })
    }
}

/// One request to the forge's API.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// The method.
    pub method: Method,
    /// The path and query under the API root, e.g. `/repos/tau-rs/arch`.
    pub path: String,
    /// The JSON body.
    pub body: Option<Value>,
}

/// The forge's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    /// The status, error statuses included.
    pub status: u16,
    /// The JSON body, `null` when empty.
    pub body: Value,
}

/// Carries requests to the forge. Error statuses are answers, not transport errors.
pub trait Transport {
    /// Send one request.
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, ForgeError>;
}

/// GitHub's REST API over `ureq`, authenticated with a [`Token`].
pub struct Ureq {
    agent: ureq::Agent,
    root: String,
    token: Token,
}

impl fmt::Debug for Ureq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ureq")
            .field("root", &self.root)
            .field("token", &self.token)
            .finish()
    }
}

impl Ureq {
    /// The transport to `https://api.github.com`.
    pub fn new(token: Token) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("arch")
            .build()
            .into();
        Ureq {
            agent,
            root: "https://api.github.com".into(),
            token,
        }
    }
}

impl Transport for Ureq {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, ForgeError> {
        let url = format!("{}{}", self.root, request.path);
        let auth = format!("Bearer {}", self.token.secret());
        let sent = match request.method {
            Method::Get => self
                .agent
                .get(&url)
                .header("Authorization", &auth)
                .header("Accept", ACCEPT)
                .header("X-GitHub-Api-Version", VERSION)
                .call(),
            Method::Post | Method::Put => {
                let body = request
                    .body
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default();
                let builder = match request.method {
                    Method::Post => self.agent.post(&url),
                    _ => self.agent.put(&url),
                };
                builder
                    .header("Authorization", &auth)
                    .header("Accept", ACCEPT)
                    .header("X-GitHub-Api-Version", VERSION)
                    .header("Content-Type", "application/json")
                    .send(body)
            }
        };
        let mut response = sent.map_err(|e| {
            ForgeError::Transport(format!("{} {}: {e}", request.method, request.path))
        })?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(|e| {
            ForgeError::Transport(format!("{} {}: {e}", request.method, request.path))
        })?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(|e| {
                ForgeError::Decode(format!("{} {}: {e}", request.method, request.path))
            })?
        };
        Ok(HttpResponse { status, body })
    }
}

const ACCEPT: &str = "application/vnd.github+json";
const VERSION: &str = "2022-11-28";
