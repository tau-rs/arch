//! The watcher (spec §6 Sync; ADR 0011, 0012): one watcher over a repository and its worktrees;
//! a burst of saves is one batch for its worktree only; a write the tool layer expects is the
//! session's, any other write is `you`; a batch applied to the analyzer updates the facts.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use arch_analyze::watch::{Batch, Change, Debounce, Watcher};
use arch_analyze::{Analyzer, Commits, Depth, Options, Recompute};
use arch_facts::{Attribution, ContentHash, Event, Store};

/// Long enough for a loaded CI runner to deliver a batch; a passing run never waits this long.
const DELIVERY: Duration = Duration::from_secs(10);

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=arch", "-c", "user.email=arch@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A committed crate at `<tmp>/repo` and a second worktree of it at `<tmp>/repo-w1` (ADR 0011).
fn repo_with_worktree(tmp: &Path) -> (PathBuf, PathBuf) {
    let repo = tmp.join("repo");
    write(
        &repo,
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&repo, "src/lib.rs", "pub mod a;\npub fn one() {}\n");
    write(&repo, "src/a.rs", "pub fn two() {}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["worktree", "add", "-q", "../repo-w1", "-b", "w1"]);
    let w1 = tmp.join("repo-w1");
    (repo.canonicalize().unwrap(), w1.canonicalize().unwrap())
}

/// Start watching and let the file-system backend settle: drop what it reports about files
/// written before the watch started.
fn watch(roots: &[PathBuf]) -> Watcher {
    let w = Watcher::new(roots, Debounce::default()).unwrap();
    while w.recv_timeout(Duration::from_millis(300)).is_some() {}
    w
}

fn paths(batch: &Batch) -> Vec<&str> {
    batch
        .changes
        .iter()
        .map(|c| c.path.to_str().unwrap())
        .collect()
}

/// The tool layer's registry of expected writes, as the watcher reads it provisionally
/// (tau-rs/arch-design#34).
fn expect_write(worktree: &Path, session: &str, element: &str, path: &str, text: &str) {
    let json = serde_json::json!({
        "session": session,
        "element": element,
        "expected": [{ "path": path, "sha256": ContentHash::of_str(text).0 }],
    });
    write(
        worktree,
        &format!(".arch/cache/tool-layer/{element}.json"),
        &json.to_string(),
    );
}

fn session(session: &str, element: &str) -> Attribution {
    serde_json::from_value(serde_json::json!({
        "by": "session", "session": session, "element": element,
    }))
    .unwrap()
}

#[test]
fn a_burst_of_saves_is_one_batch_for_its_worktree_only() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let watcher = watch(&[repo.clone(), w1.clone()]);

    // A save storm: the same file five times, another file, and files no fact comes from.
    for i in 0..5 {
        write(
            &w1,
            "src/lib.rs",
            &format!("pub mod a;\npub fn one() {{ {i}; }}\n"),
        );
    }
    write(&w1, "src/a.rs", "pub fn two() { 2; }\n");
    write(&w1, "README.md", "notes\n");
    write(&w1, "target/debug/build.rs", "// cargo output\n");
    write(&w1, ".arch/cache/x.rs", "// arch's cache\n");
    write(&w1, "src/.#lib.rs", "// an editor's lock file\n");

    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    assert_eq!(batch.worktree, w1);
    assert_eq!(paths(&batch), ["src/a.rs", "src/lib.rs"]);
    assert!(
        batch
            .changes
            .iter()
            .all(|c| !c.removed && c.attribution == Attribution::You)
    );
    assert_eq!(watcher.recv_timeout(Duration::from_millis(500)), None);
}

#[test]
fn a_directory_created_after_the_watch_starts_is_reported_and_watched() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let watcher = watch(&[repo, w1.clone()]);

    // Files written right after their directory, before a watch on it can land.
    std::fs::create_dir(w1.join("src/x")).unwrap();
    write(&w1, "src/x/mod.rs", "pub mod y;\n");
    write(&w1, "src/x/y/mod.rs", "pub fn y() {}\n");
    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    assert_eq!(batch.worktree, w1);
    assert_eq!(paths(&batch), ["src/x/mod.rs", "src/x/y/mod.rs"]);

    // The new directories are watched from then on.
    write(&w1, "src/x/y/later.rs", "pub fn later() {}\n");
    let batch = watcher.recv_timeout(DELIVERY).expect("the later batch");
    assert_eq!(paths(&batch), ["src/x/y/later.rs"]);
}

#[test]
fn a_write_the_tool_layer_expects_is_the_sessions_and_any_other_is_you() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let agent = "pub mod a;\npub fn one() { agent(); }\nfn agent() {}\n";
    expect_write(&w1, "s-1", "e-1", "src/lib.rs", agent);
    let watcher = watch(&[repo, w1.clone()]);

    // The agent's write, through the tool layer: tagged, so no `you` event.
    write(&w1, "src/lib.rs", agent);
    let batch = watcher.recv_timeout(DELIVERY).expect("the agent's batch");
    assert_eq!(
        batch.changes,
        [Change {
            path: "src/lib.rs".into(),
            removed: false,
            attribution: session("s-1", "e-1"),
        }]
    );
    let events = batch.files_changed();
    assert_eq!(events.len(), 1);
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::FilesChanged {
            attribution: Attribution::You,
            ..
        }
    )));

    // You edit the same file afterwards: its content no longer matches what the agent wrote.
    write(
        &w1,
        "src/lib.rs",
        "pub mod a;\npub fn one() { mine(); }\nfn mine() {}\n",
    );
    let batch = watcher.recv_timeout(DELIVERY).expect("your batch");
    assert_eq!(batch.changes[0].attribution, Attribution::You);
    assert_eq!(
        batch.files_changed(),
        [Event::FilesChanged {
            worktree: w1.clone(),
            files: vec!["src/lib.rs".into()],
            attribution: Attribution::You,
        }]
    );
}

#[test]
fn a_batch_with_two_writers_gives_one_files_changed_event_per_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let agent = "pub fn two() { agent(); }\nfn agent() {}\n";
    expect_write(&w1, "s-1", "e-1", "src/a.rs", agent);
    let watcher = watch(&[repo, w1.clone()]);

    write(&w1, "src/a.rs", agent);
    write(&w1, "src/lib.rs", "pub mod a;\npub fn one() { 1; }\n");
    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    let mut events = batch.files_changed();
    events.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(
        events,
        [
            Event::FilesChanged {
                worktree: w1.clone(),
                files: vec!["src/a.rs".into()],
                attribution: session("s-1", "e-1"),
            },
            Event::FilesChanged {
                worktree: w1.clone(),
                files: vec!["src/lib.rs".into()],
                attribution: Attribution::You,
            },
        ]
    );
}

#[test]
fn a_batch_applied_to_the_analyzer_updates_the_facts_and_a_removed_file_drops_out() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let options = Options {
        commits: Commits::None,
        ..Options::default()
    };
    let mut analyzer = Analyzer::open(&w1, options).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    let watcher = watch(&[repo, w1.clone()]);

    write(&w1, "src/lib.rs", "pub fn one() {}\npub fn added() {}\n");
    std::fs::remove_file(w1.join("src/a.rs")).unwrap();
    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    assert_eq!(paths(&batch), ["src/a.rs", "src/lib.rs"]);
    assert!(batch.changes[0].removed && !batch.changes[1].removed);

    let events = analyzer.apply(&batch, &mut store).unwrap();
    let Some(Event::FactsUpdated { tree, files }) = events.last() else {
        panic!("the last event is FactsUpdated: {events:?}");
    };
    assert_eq!(files, &[PathBuf::from("src/a.rs"), "src/lib.rs".into()]);
    let facts = store.facts(tree).unwrap().unwrap();
    assert!(facts.items.iter().any(|i| i.name == "added"));
    assert!(!facts.items.iter().any(|i| i.name == "two"));
}

#[test]
fn a_change_to_what_cargo_reads_alone_is_a_batch_and_reads_the_plan_again() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, _) = repo_with_worktree(tmp.path());
    let options = Options {
        commits: Commits::None,
        ..Options::default()
    };
    let mut analyzer = Analyzer::open(&repo, options).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    let watcher = watch(std::slice::from_ref(&repo));

    // `cargo update`: only the lock file changes. Then cargo's configuration appears, in a
    // hidden directory created after the watch started.
    for (path, text) in [
        (
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        ),
        (".cargo/config.toml", "[build]\njobs = 1\n"),
    ] {
        write(&repo, path, text);
        let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
        assert_eq!(paths(&batch), [path]);
        analyzer.apply(&batch, &mut store).unwrap();
        assert_eq!(analyzer.last_recompute(), Some(&Recompute::Unit), "{path}");
    }
}

#[test]
fn a_batch_for_another_worktree_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, w1) = repo_with_worktree(tmp.path());
    let mut analyzer = Analyzer::open(&repo, Options::default()).unwrap();
    let mut store = Store::in_memory().unwrap();
    let batch = Batch {
        worktree: w1,
        changes: Vec::new(),
    };
    assert!(analyzer.apply(&batch, &mut store).is_err());
}

#[test]
fn a_removed_file_leaves_rust_analyzers_view_without_degrading() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, _) = repo_with_worktree(tmp.path());
    let options = Options {
        depth: Depth::Resolved,
        commits: Commits::None,
        target_dir: Some(tmp.path().join("target")),
        ..Options::default()
    };
    let mut analyzer = Analyzer::open(&repo, options).unwrap();
    assert_eq!(analyzer.degraded(), None);
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    let watcher = watch(std::slice::from_ref(&repo));

    write(
        &repo,
        "src/lib.rs",
        "pub fn one() { three() }\nfn three() {}\n",
    );
    std::fs::remove_file(repo.join("src/a.rs")).unwrap();
    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    let events = analyzer.apply(&batch, &mut store).unwrap();
    let Some(Event::FactsUpdated { tree, .. }) = events.last() else {
        panic!("{events:?}");
    };
    let facts = store.facts(tree).unwrap().unwrap();
    assert!(
        facts.analyzer.degraded.is_empty(),
        "{:?}",
        facts.analyzer.degraded
    );
    assert!(!facts.items.iter().any(|i| i.name == "two"));
    assert!(facts.links.iter().any(|l| l.from.ends_with("one#fn")
        && l.confidence == arch_facts::Confidence::Resolved));
}

#[test]
fn at_resolved_depth_a_file_created_after_load_is_resolved_like_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, _) = repo_with_worktree(tmp.path());
    let options = Options {
        depth: Depth::Resolved,
        commits: Commits::None,
        target_dir: Some(tmp.path().join("target")),
        ..Options::default()
    };
    let mut analyzer = Analyzer::open(&repo, options).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    let watcher = watch(std::slice::from_ref(&repo));

    write(&repo, "src/b.rs", "pub fn three() { crate::one() }\n");
    write(
        &repo,
        "src/lib.rs",
        "pub mod a;\npub mod b;\npub fn one() { b::three() }\n",
    );
    let batch = watcher.recv_timeout(DELIVERY).expect("one batch");
    assert_eq!(paths(&batch), ["src/b.rs", "src/lib.rs"]);
    let events = analyzer.apply(&batch, &mut store).unwrap();
    let Some(Event::FactsUpdated { tree, .. }) = events.last() else {
        panic!("{events:?}");
    };
    let facts = store.facts(tree).unwrap().unwrap();
    let call = |from: &str, to: &str| {
        facts
            .links
            .iter()
            .find(|l| l.from == from && l.to == arch_facts::Target::Item(to.into()))
            .map(|l| l.confidence)
    };
    let resolved = Some(arch_facts::Confidence::Resolved);
    assert_eq!(
        (
            call("demo::b::three#fn", "demo::one#fn"),
            call("demo::one#fn", "demo::b::three#fn")
        ),
        (resolved, resolved),
        "{:#?}",
        facts.links
    );
    assert!(
        facts.analyzer.degraded.is_empty(),
        "{:?}",
        facts.analyzer.degraded
    );
}
