//! The app method set and its published schema, `schemas/arch-api.json` (ADR 0034).
//!
//! The schema is an OpenRPC 1.3 document generated from the types below: methods take their
//! params by name, results and events refer to `components.schemas`, pushed events are listed in
//! `x-arch-events`. `info.version` is [`API_SCHEMA_VERSION`], semver: an addition bumps the
//! minor, a break bumps the major, even at 0 (ADR 0034 §4).

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Version of the app API, `info.version` of `schemas/arch-api.json`.
pub const API_SCHEMA_VERSION: &str = "0.1.0";

/// `initialize` params: who is calling, and the schema version it was generated from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Who is calling, free text (`arch-app`, `arch-fixtures`, a test).
    pub client: String,
    /// The schema version the client was generated from; the engine answers whatever it is.
    pub schema_version: Option<String>,
}

/// `initialize` result: the engine's version and the schema version it was built with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// The engine's semver, as `arch --version` prints it after `arch `.
    pub engine_version: String,
    /// `info.version` of the `schemas/arch-api.json` the engine was built with.
    pub schema_version: String,
}

/// The OpenRPC 1.3 document published as `schemas/arch-api.json`.
pub fn openrpc_document() -> Value {
    let mut generator = SchemaSettings::draft07()
        .with(|s| {
            s.definitions_path = "/components/schemas".into();
            s.meta_schema = None;
        })
        .into_generator();
    let methods = vec![method::<InitializeParams, InitializeResult>(
        &mut generator,
        "initialize",
        "Readiness and version handshake",
        "Always answered, whatever schema version the client sends: comparing versions is the \
         client's job, and a mismatch is a status, not a refused connection (ADR 0034 §3).",
    )];
    json!({
        "openrpc": "1.3.2",
        "info": {
            "title": "arch-api",
            "version": API_SCHEMA_VERSION,
            "description": "The arch engine's app API: one method set served by `arch serve` \
                            over a per-repo unix socket, one JSON-RPC 2.0 message per line \
                            (tau-rs/arch-design ADR 0034).",
            "license": { "name": "MIT OR Apache-2.0" },
        },
        "methods": methods,
        "components": { "schemas": generator.take_definitions(true) },
        "x-arch-events": [],
    })
}

/// One method entry: params by name from `P`'s properties, the result as a `$ref` to `R`.
fn method<P: JsonSchema, R: JsonSchema>(
    generator: &mut SchemaGenerator,
    name: &str,
    summary: &str,
    description: &str,
) -> Value {
    let params = P::json_schema(generator);
    let required: Vec<&str> = params
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let params: Vec<Value> = params
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::iter)
        .map(|(param, schema)| {
            let mut schema = schema.clone();
            let description = schema.as_object_mut().and_then(|s| s.remove("description"));
            let mut descriptor = json!({
                "name": param,
                "required": required.contains(&param.as_str()),
                "schema": schema,
            });
            if let Some(text) = description {
                descriptor["description"] = text;
            }
            descriptor
        })
        .collect();
    json!({
        "name": name,
        "summary": summary,
        "description": description,
        "paramStructure": "by-name",
        "params": params,
        "result": { "name": R::schema_name(), "schema": generator.subschema_for::<R>() },
    })
}

/// `initialize`: the engine's version and [`API_SCHEMA_VERSION`], whatever the client sent.
pub fn initialize(_params: InitializeParams) -> InitializeResult {
    InitializeResult {
        engine_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: API_SCHEMA_VERSION.into(),
    }
}

/// A JSON-RPC 2.0 error: one of the standard codes, or arch's own in `-32000..=-32099`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// The error code.
    pub code: i64,
    /// What went wrong, for a person.
    pub message: String,
}

impl RpcError {
    const PARSE: i64 = -32700;
    const INVALID_REQUEST: i64 = -32600;
    const METHOD_NOT_FOUND: i64 = -32601;
    const INVALID_PARAMS: i64 = -32602;

    fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError {
            code,
            message: message.into(),
        }
    }
}

/// Run one method of the app method set: the handlers every transport shares (ADR 0034).
pub fn dispatch(method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => {
            let params = serde_json::from_value(params)
                .map_err(|e| RpcError::new(RpcError::INVALID_PARAMS, format!("initialize: {e}")))?;
            Ok(serde_json::to_value(initialize(params)).expect("the result serializes"))
        }
        other => Err(RpcError::new(
            RpcError::METHOD_NOT_FOUND,
            format!("no method {other:?} in arch-api {API_SCHEMA_VERSION}"),
        )),
    }
}

/// One line of the socket protocol in, the reply line out: `None` for a blank line or a
/// notification, which get no reply (ADR 0034 §2).
pub fn handle_line(line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let reply = match serde_json::from_str::<Value>(line) {
        Err(e) => error_reply(Value::Null, RpcError::new(RpcError::PARSE, e.to_string())),
        Ok(request) => {
            let id = request.get("id").cloned();
            let method = request.get("method").and_then(Value::as_str);
            match (request.get("jsonrpc").and_then(Value::as_str), method, id) {
                (Some("2.0"), Some(method), Some(id)) => {
                    let params = request.get("params").cloned().unwrap_or(Value::Null);
                    match dispatch(method, params) {
                        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                        Err(e) => error_reply(id, e),
                    }
                }
                // a notification: nothing the client sends is one yet, and none gets a reply
                (Some("2.0"), Some(_), None) => return None,
                (_, _, id) => error_reply(
                    id.unwrap_or(Value::Null),
                    RpcError::new(
                        RpcError::INVALID_REQUEST,
                        "not a JSON-RPC 2.0 request (one object per line, no batches)",
                    ),
                ),
            }
        }
    };
    Some(reply.to_string())
}

fn error_reply(id: Value, e: RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } })
}
