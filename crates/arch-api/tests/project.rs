//! The session engine's port (`arch_session::Project`) over arch-api: `arch check` for the gate,
//! the facts for the context pack.

mod common;

use arch_api::ArchProject;
use arch_session::Project;

#[test]
fn the_port_checks_and_reads_the_facts_of_a_worktree() {
    let repo = common::smallsvc_copy();
    let doc = ArchProject.check(repo.path()).unwrap();
    assert_eq!(doc["schema_version"], 0);
    assert_eq!(doc["summary"]["blocking"], 0);
    let facts = ArchProject.facts(repo.path()).unwrap();
    assert_eq!(
        facts.repo.name,
        repo.path().file_name().unwrap().to_str().unwrap()
    );
    assert!(!facts.items.is_empty());
}

#[test]
fn a_worktree_without_dot_arch_is_a_check_error_for_the_gate() {
    let repo = common::smallsvc_copy();
    std::fs::remove_dir_all(repo.path().join(".arch")).unwrap();
    let e = ArchProject.check(repo.path()).unwrap_err();
    assert!(e.contains("arch init"), "{e}");
}
