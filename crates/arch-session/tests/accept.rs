//! Accept on a temp repository (issue #47 "Done when", first item).

use std::path::Path;
use std::process::Command;

use arch_facts::*;
use arch_session::{AcceptOptions, Error, accept};
use pretty_assertions::assert_eq;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `<tmp>/smallsvc` with one commit, and its store holding a two-element draft.
fn repo_with_draft() -> (tempfile::TempDir, std::path::PathBuf, Store, Plan) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("smallsvc");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    std::fs::write(repo.join(".gitignore"), ".arch/cache/\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&repo.join(".arch")).unwrap();
    let mut plan = Plan::new(SessionId::new("3c9e1f0a"), "add a refund flow");
    let e1 = plan.add_element("add Refund", "src/refund.rs").id.clone();
    plan.add_element("wire it", "src/lib.rs");
    plan.elements[1].depends_on.push(e1);
    store.plan_draft_put(&plan).unwrap();
    (tmp, repo, store, plan)
}

#[test]
fn accept_creates_branch_worktree_three_files_and_one_commit() {
    let (tmp, repo, store, plan) = repo_with_draft();
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let planner = vec![ThreadEntry::new(
        ThreadAuthor::Planner,
        ThreadEvent::Changed {
            what: Some("2 elements".into()),
        },
    )];

    let session = accept(
        &repo,
        &store,
        &plan.session,
        &planner,
        &AcceptOptions::default(),
    )
    .unwrap();

    // The branch, from HEAD, and the worktree at <parent>/<repo>-w1 (ADR 0011).
    let worktree = tmp.path().canonicalize().unwrap().join("smallsvc-w1");
    assert_eq!(session.worktree.as_deref(), Some(worktree.as_path()));
    assert_eq!(session.branch.as_deref(), Some("arch/3c9e1f0a"));
    assert_eq!(session.base.as_deref(), Some(head.as_str()));
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "arch/3c9e1f0a"
    );
    assert_eq!(
        git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main",
        "the repo stays put"
    );

    // The three files.
    let dir = ArchDir::of_repo(&worktree).session(&plan.session);
    let written = dir.read_plan().unwrap().unwrap();
    assert_eq!(written.elements, plan.elements);
    assert_eq!(written.groups.len(), 2, "shaped: E2 depends on E1");
    assert_eq!(written.groups[0].gate.commands, ["cargo test --workspace"]);
    let thread = dir.read_thread().unwrap();
    assert_eq!(thread[0], planner[0], "the planner's thread comes first");
    assert_eq!(thread[1].author, ThreadAuthor::Arch);
    assert_eq!(dir.read_session().unwrap(), Some(session.clone()));
    assert_eq!(session.state, SessionState::Running);
    assert_eq!(session.cursor.todo, written.groups[0].elements);

    // One commit on top of the base, holding exactly the three files.
    assert_eq!(
        git(
            &worktree,
            &["rev-list", "--count", &format!("{head}..HEAD")]
        ),
        "1"
    );
    assert_eq!(
        git(
            &worktree,
            &[
                "log",
                "-1",
                "--format=%s%n%(trailers:key=Arch-Session,valueonly)"
            ]
        ),
        "chore(arch): plan add a refund flow\n3c9e1f0a"
    );
    assert_eq!(
        git(&worktree, &["show", "--name-only", "--format=", "HEAD"]),
        ".arch/sessions/3c9e1f0a/plan.toml\n.arch/sessions/3c9e1f0a/session.toml\n.arch/sessions/3c9e1f0a/thread.jsonl"
    );
    assert_eq!(
        git(&worktree, &["status", "--porcelain"]),
        "",
        "nothing left uncommitted"
    );

    // The draft left the cache (ADR 0020).
    assert_eq!(store.plan_draft("3c9e1f0a").unwrap(), None);
}

#[test]
fn the_next_session_takes_the_lowest_free_worktree_number() {
    let (tmp, repo, store, mut plan) = repo_with_draft();
    std::fs::create_dir(tmp.path().join("smallsvc-w1")).unwrap();
    let parent = tmp.path().join("trees");
    std::fs::create_dir(&parent).unwrap();

    let s = accept(&repo, &store, &plan.session, &[], &AcceptOptions::default()).unwrap();
    assert!(s.worktree.unwrap().ends_with("smallsvc-w2"));

    plan.session = SessionId::new("second");
    store.plan_draft_put(&plan).unwrap();
    let options = AcceptOptions {
        worktree_parent: Some(parent.clone()),
        delegate: false,
        name: Some("by hand".into()),
        ..AcceptOptions::default()
    };
    let s = accept(&repo, &store, &plan.session, &[], &options).unwrap();
    assert_eq!(
        s.worktree.as_deref(),
        Some(parent.join("smallsvc-w1").as_path()),
        "configurable parent"
    );
    assert_eq!(
        s.state,
        SessionState::Yours,
        "accept without delegate: a locked you session"
    );
    assert!(s.cursor.todo.is_empty());
    assert_eq!(s.name, "by hand");
}

#[test]
fn accept_without_a_draft_or_on_a_taken_branch_fails_and_leaves_no_worktree() {
    let (tmp, repo, store, plan) = repo_with_draft();
    let e = accept(
        &repo,
        &store,
        &SessionId::new("nope"),
        &[],
        &AcceptOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(e, Error::NoDraft(_)), "{e}");

    git(&repo, &["branch", "arch/3c9e1f0a"]);
    let e = accept(&repo, &store, &plan.session, &[], &AcceptOptions::default()).unwrap_err();
    assert!(
        e.to_string().contains("arch/3c9e1f0a already exists"),
        "{e}"
    );
    assert!(!tmp.path().join("smallsvc-w1").exists());
    assert!(
        store.plan_draft("3c9e1f0a").unwrap().is_some(),
        "the draft stays in the cache"
    );
}

#[test]
fn a_cyclic_draft_is_refused_before_anything_is_created() {
    let (tmp, repo, store, mut plan) = repo_with_draft();
    let e2 = plan.elements[1].id.clone();
    plan.elements[0].depends_on.push(e2);
    store.plan_draft_put(&plan).unwrap();
    let e = accept(&repo, &store, &plan.session, &[], &AcceptOptions::default()).unwrap_err();
    assert!(matches!(e, Error::Cycle { .. }), "{e}");
    assert!(!tmp.path().join("smallsvc-w1").exists());
}
