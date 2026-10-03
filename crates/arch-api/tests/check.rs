//! `check` end to end on the sample service, and the published output schema.

mod common;

use arch_api::{CheckOutput, Error, Level, check};

#[test]
fn the_sample_service_passes_with_its_planted_violation_allowed() {
    let repo = common::smallsvc_copy();
    let output = check(repo.path()).unwrap();

    assert_eq!(
        output.repo,
        repo.path().file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(output.summary.blocking, 0);
    assert_eq!(output.exit_code(), 0);
    // the one planted violation (README: "one finding behind an allow") shows as two links
    assert_eq!(output.summary.allowed, 2);
    let allowed: Vec<_> = output
        .findings
        .iter()
        .filter(|f| f.allowed.is_some())
        .collect();
    assert!(
        allowed
            .iter()
            .all(|f| f.site == "src/app/notify.rs::NotifyCustomer::deliver")
    );
    // syntax-level facts are guessed: nothing may block (ADR 0009)
    assert!(output.findings.iter().all(|f| f.level == Level::Warn));
    assert!(!output.degraded.is_empty());
}

#[test]
fn a_repo_without_dot_arch_is_a_tool_error_not_a_clean_check() {
    let repo = common::smallsvc_copy();
    std::fs::remove_dir_all(repo.path().join(".arch")).unwrap();
    assert!(matches!(check(repo.path()), Err(Error::NotInitialized(_))));
}

#[test]
fn a_blocking_finding_exits_one() {
    let repo = common::smallsvc_copy();
    let mut output = check(repo.path()).unwrap();
    assert_eq!(output.exit_code(), 0);
    output.summary.blocking = 1;
    assert_eq!(output.exit_code(), 1);
}

fn pretty(v: &serde_json::Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

/// `ARCH_UPDATE_SCHEMA=1 cargo test -p arch-api` rewrites the file.
#[test]
fn committed_check_schema_matches_the_types() {
    let path = common::repo_root().join("schemas/check.schema.json");
    let generated = pretty(&CheckOutput::json_schema());
    if std::env::var_os("ARCH_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "schemas/check.schema.json is out of date; run ARCH_UPDATE_SCHEMA=1 cargo test -p arch-api"
    );
}

#[test]
fn real_output_validates_against_the_schema() {
    let repo = common::smallsvc_copy();
    let output = serde_json::to_value(check(repo.path()).unwrap()).unwrap();
    let schema = CheckOutput::json_schema();
    let validator = jsonschema::draft202012::new(&schema).expect("schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(&output)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}
