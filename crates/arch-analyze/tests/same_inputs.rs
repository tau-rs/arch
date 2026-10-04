//! `same_inputs_same_facts` (ADR 0030): a cold analysis of the same commit in two checkouts at
//! different paths, each with its own `target/` and store, gives byte-identical facts. The
//! pinned smallsvc pins its target, so this holds across machines too: CI runs it on Linux and
//! on macOS and compares the two outputs (`ARCH_FACTS_OUT`), byte for byte.
//!
//! `incremental_equals_cold` (ADR 0002) checks incremental against cold on one machine; this
//! checks cold against cold, wherever the checkout and whatever the machine. rust-analyzer loads
//! once per checkout, so it is `#[ignore]`d and runs in release in CI:
//!
//! ```text
//! cargo test --release -p arch-analyze --test same_inputs -- --ignored
//! ```

use std::path::Path;

use arch_analyze::{Commits, Depth, Options, analyze};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

/// The facts of a fresh checkout of smallsvc at `place` (under `tmp`), analysed cold with its
/// own target directory, as pretty JSON.
fn cold(tmp: &Path, place: &str) -> String {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        fixture.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        fixture.display()
    );
    let repo = tmp.join(place).join("smallsvc");
    copy_dir(&fixture, &repo);
    let golden = std::fs::read_to_string(fixture.join("../../golden/smallsvc/facts.json")).unwrap();
    let golden: serde_json::Value = serde_json::from_str(&golden).unwrap();
    let options = Options {
        depth: Depth::Resolved,
        repo_name: Some("smallsvc".into()),
        commit: golden["repo"]["commit"].as_str().map(str::to_string),
        commits: Commits::None,
        target_dir: Some(tmp.join(place).join("target")),
    };
    let facts = analyze(&repo, &options).unwrap();
    assert!(
        facts.analyzer.degraded.is_empty(),
        "{place}: {:?}",
        facts.analyzer.degraded
    );
    serde_json::to_string_pretty(&facts).unwrap()
}

#[test]
#[ignore = "loads rust-analyzer twice; release, in CI"]
fn same_inputs_same_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let here = cold(tmp.path(), "a");
    let there = cold(tmp.path(), "b/deeper/elsewhere");
    assert!(
        here == there,
        "two checkouts of one commit disagree; first difference near byte {}",
        here.bytes()
            .zip(there.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(here.len().min(there.len()))
    );
    let facts: serde_json::Value = serde_json::from_str(&here).unwrap();
    assert_eq!(
        (
            &facts["analyzer"]["target"],
            &facts["analyzer"]["target_pinned"]
        ),
        (
            &serde_json::json!("x86_64-unknown-linux-gnu"),
            &serde_json::json!(true)
        ),
        "smallsvc pins its target, so every machine analyses for the same one"
    );
    // For CI to compare machines.
    if let Some(out) = std::env::var_os("ARCH_FACTS_OUT") {
        std::fs::write(out, &here).unwrap();
    }
}
