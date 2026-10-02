//! `schemas/facts.schema.json` is generated from the types; this test keeps the file honest.
//! `ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts` rewrites it.

use std::path::PathBuf;

use arch_facts::Facts;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pretty(v: &serde_json::Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

#[test]
fn committed_schema_matches_the_types() {
    let path = repo_root().join("schemas/facts.schema.json");
    let generated = pretty(&Facts::json_schema());
    if std::env::var_os("ARCH_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts",
            path.display()
        )
    });
    assert!(
        committed == generated,
        "schemas/facts.schema.json is out of date; run ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts"
    );
}

#[test]
fn examples_validate_and_round_trip() {
    let schema = Facts::json_schema();
    let validator = jsonschema::draft202012::new(&schema).expect("schema compiles");
    let dir = repo_root().join("schemas/examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
        let facts: Facts = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&facts).unwrap(),
            value,
            "{}: not canonical",
            path.display()
        );
        seen += 1;
    }
    assert!(seen > 0, "no examples under {}", dir.display());
}
