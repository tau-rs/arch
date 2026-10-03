//! The `arch` binary's exit codes and output formats.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn arch(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn smallsvc() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        dir.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        dir.display()
    );
    dir
}

#[test]
fn check_json_is_the_schema_document_and_exits_zero_on_warnings() {
    let out = arch(
        &["check", smallsvc().to_str().unwrap(), "--format", "json"],
        Path::new("."),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema_version"], 0);
    assert_eq!(doc["summary"]["blocking"], 0);
    assert_eq!(doc["summary"]["allowed"], 2);
    assert_eq!(doc["findings"][0]["origin"], "core");
}

#[test]
fn check_human_names_rule_site_and_witness() {
    let out = arch(&["check"], &smallsvc());
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("allowed domain must not depend-on driven"),
        "{text}"
    );
    assert!(
        text.contains("src/app/notify.rs::NotifyCustomer::deliver"),
        "{text}"
    );
    assert!(text.contains("at src/app/notify.rs:68"), "{text}");
    assert!(text.contains("0 blocking"), "{text}");
}

#[test]
fn a_tool_error_exits_two_with_a_message() {
    let empty = tempfile::tempdir().unwrap();
    let out = arch(&["check"], empty.path());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("arch:"));
}
