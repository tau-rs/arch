//! Runs the harness over every `golden/<repo>/` directory.
//!
//! A repo with no golden data reports "awaiting data" and passes. A repo with a `facts.json` is loaded
//! through `load_golden` and must satisfy every invariant that facts alone can decide; the ones that
//! read views hold trivially until `views/` exists.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use arch_fixtures_checks::{check_all, load_golden, LoadError};

/// The branch a change is judged against: its merge base with this is "what the change edits".
const BASE: &str = "origin/main";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn golden_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(repo_root().join("golden"))
        .expect("golden/ exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "golden/ has no repo directories");
    dirs
}

fn dir_name(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn every_golden_repo_satisfies_the_invariants() {
    let mut failures = Vec::new();
    for dir in golden_dirs() {
        let name = dir_name(&dir);
        match load_golden(&dir) {
            Ok(input) => {
                let report = check_all(&input);
                if !report.is_clean() {
                    failures.push(format!("{name}:\n{report}"));
                }
            }
            Err(LoadError::AwaitingData) => eprintln!("{name}: awaiting data"),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn git(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Current,
    Stale(String),
    Fail(String),
}

/// An in-repo fixture's golden records, in `repo.commit`, the last commit touching `repos/<repo>/`.
/// A squash merge's ID is unknown until it lands, so a fixture edit and its golden bump are two PRs
/// (#11). Only a change that edits `golden/<repo>/` must be exact; elsewhere, including main between
/// those two merges, a stale value is a warning.
fn judge(
    name: &str,
    recorded: &str,
    last: &str,
    edits_golden: bool,
    edits_fixture: bool,
) -> Verdict {
    if edits_golden && edits_fixture {
        return Verdict::Fail(format!(
            "{name}: change edits both repos/{name}/ and golden/{name}/; its squash ID is unknown \
             until merge, so update the golden in a follow-up PR"
        ));
    }
    if recorded == last {
        return Verdict::Current;
    }
    let msg = format!(
        "{name}: golden repo.commit {recorded} is not the last commit touching repos/{name}/ ({last})"
    );
    if edits_golden {
        Verdict::Fail(msg)
    } else {
        Verdict::Stale(format!("{msg}; bump it in a golden-only PR"))
    }
}

/// Bypasses libtest's capture so the warning shows in a passing run; an annotation on GitHub Actions.
#[allow(clippy::explicit_write)] // `println!` would be captured; this must not be.
fn warn(msg: &str) {
    let prefix = if std::env::var_os("GITHUB_ACTIONS").is_some() {
        "::warning::"
    } else {
        "warning: "
    };
    writeln!(std::io::stdout(), "{prefix}{msg}").unwrap();
}

#[test]
fn golden_repo_commit_is_the_last_fixture_commit() {
    // A shallow clone's oldest commit "touches" every file, so `git log -- path` would lie.
    assert_eq!(
        git(&["rev-parse", "--is-shallow-repository"]),
        "false",
        "shallow clone: repo.commit needs full history (actions/checkout fetch-depth: 0)"
    );
    let base = git(&["merge-base", "HEAD", BASE]);
    // Against the working tree, so an uncommitted golden edit is already judged strictly.
    let changed = git(&["diff", "--name-only", &base]);
    let edits = |prefix: &str| changed.lines().any(|path| path.starts_with(prefix));
    let mut failures = Vec::new();
    for dir in golden_dirs() {
        let name = dir_name(&dir);
        let Ok(text) = std::fs::read_to_string(dir.join("facts.json")) else {
            continue;
        };
        let fixture = format!("repos/{name}");
        // Submodules are pinned commits, not trees in this repo; this rule covers in-repo fixtures.
        if !git(&["ls-tree", "HEAD", &fixture]).starts_with("040000 tree ") {
            continue;
        }
        let facts: serde_json::Value = serde_json::from_str(&text).expect("facts.json parses");
        let recorded = facts["repo"]["commit"]
            .as_str()
            .expect("repo.commit is a string");
        let last = git(&["log", "-1", "--format=%H", "HEAD", "--", &fixture]);
        match judge(
            &name,
            recorded,
            &last,
            edits(&format!("golden/{name}/")),
            edits(&format!("{fixture}/")),
        ) {
            Verdict::Current => {}
            Verdict::Stale(msg) => warn(&msg),
            Verdict::Fail(msg) => failures.push(msg),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn judge_is_strict_only_on_golden_edits() {
    let stale = |v: Verdict| matches!(v, Verdict::Stale(_));
    let fail = |v: Verdict| matches!(v, Verdict::Fail(_));
    assert_eq!(judge("s", "a", "a", true, false), Verdict::Current);
    assert_eq!(judge("s", "a", "a", false, false), Verdict::Current);
    assert!(fail(judge("s", "old", "new", true, false)));
    assert!(stale(judge("s", "old", "new", false, false)));
    assert!(stale(judge("s", "old", "new", false, true)));
    assert!(fail(judge("s", "a", "a", true, true)));
}
