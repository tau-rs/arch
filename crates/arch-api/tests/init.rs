//! `init` end to end on a copy of the sample service with its `.arch/` removed.

mod common;

use arch_api::{ColumnRule, Error, InitOptions, check, init};

fn bare_copy() -> tempfile::TempDir {
    let repo = common::smallsvc_copy();
    std::fs::remove_dir_all(repo.path().join(".arch")).unwrap();
    repo
}

#[test]
fn init_writes_areas_rules_and_the_gitignore_line_in_one_commit() {
    let repo = bare_copy();
    // the fixture already ignores the cache; drop the line so init has to add it
    let gitignore = repo.path().join(".gitignore");
    let kept: String = std::fs::read_to_string(&gitignore)
        .unwrap()
        .lines()
        .filter(|l| *l != ".arch/cache/")
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&gitignore, kept).unwrap();
    common::git_init(repo.path());
    let before = common::git(repo.path(), &["rev-list", "--count", "HEAD"]);

    let outcome = init(repo.path(), &InitOptions::default()).unwrap();

    assert_eq!(outcome.areas.rule, Some(ColumnRule::Hexagon));
    assert_eq!(outcome.areas.main_bin.as_deref(), Some("orderly"));
    let names: Vec<&str> = outcome
        .areas
        .areas
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    for expected in ["http", "worker", "domain", "ports", "postgres", "stripe"] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }

    // one commit, holding exactly the files init wrote
    let after = common::git(repo.path(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(
        after.parse::<u32>().unwrap(),
        before.parse::<u32>().unwrap() + 1
    );
    assert_eq!(
        outcome.commit.as_deref(),
        Some(common::git(repo.path(), &["rev-parse", "HEAD"]).as_str())
    );
    let mut files: Vec<String> =
        common::git(repo.path(), &["show", "--name-only", "--format=", "HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
    files.sort();
    assert_eq!(files, vec![".arch/areas.toml", ".arch/rules", ".gitignore"]);
    assert_eq!(common::git(repo.path(), &["status", "--porcelain"]), "");
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert_eq!(
        gitignore.lines().filter(|l| *l == ".arch/cache/").count(),
        1
    );

    // what it wrote is what check reads
    let output = check(repo.path()).unwrap();
    assert_eq!(output.summary.blocking, 0);
}

#[test]
fn a_gitignore_that_already_has_the_line_is_left_alone() {
    let repo = bare_copy();
    let before = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(
        before.lines().any(|l| l == ".arch/cache/"),
        "fixture changed: {before}"
    );
    let outcome = init(repo.path(), &InitOptions { commit: false }).unwrap();
    assert_eq!(outcome.written.len(), 2);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore")).unwrap(),
        before
    );
}

#[test]
fn init_runs_once() {
    let repo = bare_copy();
    init(repo.path(), &InitOptions { commit: false }).unwrap();
    assert!(matches!(
        init(repo.path(), &InitOptions { commit: false }),
        Err(Error::AlreadyInitialized(_))
    ));
}

#[test]
fn no_commit_writes_the_files_and_leaves_git_alone() {
    let repo = bare_copy();
    common::git_init(repo.path());
    let head = common::git(repo.path(), &["rev-parse", "HEAD"]);

    let outcome = init(repo.path(), &InitOptions { commit: false }).unwrap();

    assert_eq!(outcome.commit, None);
    assert!(repo.path().join(".arch/areas.toml").is_file());
    assert!(repo.path().join(".arch/rules").is_file());
    assert_eq!(common::git(repo.path(), &["rev-parse", "HEAD"]), head);
}

#[test]
fn committing_outside_a_git_repository_says_so_before_writing_anything() {
    let repo = bare_copy();
    let err = init(repo.path(), &InitOptions::default()).unwrap_err();
    assert!(matches!(err, Error::Git(_)), "{err}");
    assert!(err.to_string().contains("--no-commit"));
    assert!(!repo.path().join(".arch").exists());
}
