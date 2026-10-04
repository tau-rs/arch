//! The published app API schema, `schemas/arch-api.json` (ADR 0034).

mod common;

use arch_api::rpc::{API_SCHEMA_VERSION, InitializeResult, openrpc_document};
use jsonschema::{Draft, Registry};
use serde_json::{Value, json};

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

fn vendored(name: &str) -> Value {
    let path = common::repo_root()
        .join("crates/arch-api/tests/openrpc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The OpenRPC 1.3 meta-schema, resolved offline (`tests/openrpc/README.md`).
fn openrpc_errors(doc: &Value) -> Vec<String> {
    let registry = Registry::new()
        .add(
            "https://meta.json-schema.tools",
            Draft::Draft7.create_resource(vendored("json-schema-tools-meta-schema-1.8.0.json")),
        )
        .unwrap()
        .prepare()
        .unwrap();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft7)
        .with_registry(&registry)
        .build(&vendored("open-rpc-meta-schema-1.14.9.json"))
        .expect("the meta-schema compiles");
    validator.iter_errors(doc).map(|e| e.to_string()).collect()
}

/// `ARCH_UPDATE_SCHEMA=1 cargo test -p arch-api` rewrites the file.
#[test]
fn committed_api_schema_matches_the_types() {
    let path = common::repo_root().join("schemas/arch-api.json");
    let generated = pretty(&openrpc_document());
    if std::env::var_os("ARCH_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "schemas/arch-api.json is out of date; run ARCH_UPDATE_SCHEMA=1 cargo test -p arch-api"
    );
}

#[test]
fn arch_api_schema_is_an_openrpc_1_3_document() {
    let errors = openrpc_errors(&openrpc_document());
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn the_meta_schema_check_rejects_a_document_without_info() {
    let mut doc = openrpc_document();
    doc.as_object_mut().unwrap().remove("info");
    assert!(!openrpc_errors(&doc).is_empty());
}

#[test]
fn version_0_1_0_holds_initialize_and_no_events() {
    let doc = openrpc_document();
    assert_eq!(doc["openrpc"], "1.3.2");
    assert_eq!(doc["info"]["version"], API_SCHEMA_VERSION);
    assert_eq!(API_SCHEMA_VERSION, "0.1.0");
    let names: Vec<&str> = doc["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["initialize"]);
    assert_eq!(doc["x-arch-events"], json!([]));
}

#[test]
fn initialize_takes_client_and_an_optional_schema_version_by_name() {
    let doc = openrpc_document();
    let init = &doc["methods"][0];
    assert_eq!(init["paramStructure"], "by-name");
    let params: Vec<(&str, bool)> = init["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["required"] == true))
        .collect();
    assert_eq!(params, [("client", true), ("schemaVersion", false)]);
    assert_eq!(
        init["result"]["schema"]["$ref"],
        "#/components/schemas/InitializeResult"
    );
}

#[test]
fn an_initialize_result_validates_against_its_published_schema() {
    let doc = openrpc_document();
    let schema = &doc["components"]["schemas"]["InitializeResult"];
    let result = serde_json::to_value(InitializeResult {
        engine_version: "0.1.0".into(),
        schema_version: API_SCHEMA_VERSION.into(),
    })
    .unwrap();
    assert_eq!(
        result,
        json!({ "engineVersion": "0.1.0", "schemaVersion": "0.1.0" })
    );
    let validator = jsonschema::draft7::new(schema).unwrap();
    assert!(validator.is_valid(&result));
    assert!(!validator.is_valid(&json!({ "engineVersion": "0.1.0" })));
}
