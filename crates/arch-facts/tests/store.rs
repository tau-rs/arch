//! The store: per-file deltas named by facts key, trees, pointers, cache, drafts
//! (ADR 0001, 0002, 0020).

use std::path::{Path, PathBuf};

use arch_facts::*;
use pretty_assertions::assert_eq;

fn item(id: &str, file: &str) -> Item {
    Item {
        id: id.into(),
        kind: ItemKind::Fn,
        name: id.rsplit("::").next().unwrap().to_string(),
        file: file.into(),
        span: Span {
            line: 1,
            col: 1,
            end_line: 2,
            end_col: 1,
        },
        crate_name: "smallsvc".into(),
        module: "ship".into(),
        parent: None,
        visibility: Visibility::Pub,
        reexported: false,
        flags: ItemFlags::default(),
        scope: "unit:smallsvc".into(),
    }
}

fn head(commit: &str) -> TreeHead {
    TreeHead {
        repo: Repo {
            name: "smallsvc".into(),
            commit: commit.into(),
            unit: Unit {
                id: "unit:smallsvc".into(),
                main_target: "bin:smallsvc".into(),
            },
        },
        analyzer: Analyzer {
            name: "arch-analyze".into(),
            version: "0.1.0".into(),
            degraded: vec![],
        },
        crates: vec![Crate {
            name: "smallsvc".into(),
            status: CrateStatus::Analyzed,
            targets: vec!["bin:smallsvc".into()],
        }],
        assembled: Default::default(),
    }
}

fn file_facts(path: &str, content: &str, ids: &[&str]) -> FileFacts {
    let mut f = FileFacts::empty(path, ContentHash::of_str(content));
    f.items = ids.iter().map(|i| item(i, path)).collect();
    f
}

/// The tree entry of a delta in a package whose id is `package`.
fn tf(f: &FileFacts, package: &str) -> TreeFile {
    TreeFile {
        path: f.path.clone(),
        file_hash: f.file_hash.clone(),
        facts_key: FactsKey::of(&f.path, &f.file_hash, package),
    }
}

#[test]
fn facts_assemble_from_deltas_and_a_rebase_reuses_them_by_facts_key() {
    let mut store = Store::in_memory().unwrap();
    let a = file_facts("src/a.rs", "a v1", &["smallsvc::a::f"]);
    let b = file_facts("src/b.rs", "b v1", &["smallsvc::b::g"]);
    let files = vec![tf(&a, "p1"), tf(&b, "p1")];
    store.put_file_facts(&files[0].facts_key, &a).unwrap();

    let c1 = TreeKey::commit("c1");
    store
        .put_tree(&c1, None, &files, &head("c1"), &[], &[])
        .unwrap();
    assert!(store.has_tree(&c1).unwrap());

    // b is not computed yet: the analyzer is told exactly that.
    assert_eq!(
        store.missing_file_facts(&c1).unwrap(),
        vec![files[1].clone()]
    );
    assert!(!store.has_file_facts(&files[1].facts_key).unwrap());
    store.put_file_facts(&files[1].facts_key, &b).unwrap();
    assert!(store.missing_file_facts(&c1).unwrap().is_empty());
    assert_eq!(
        store.file_facts(&files[1].facts_key).unwrap(),
        Some(b.clone())
    );

    let facts = store.facts(&c1).unwrap().unwrap();
    assert_eq!(facts.items.len(), 2);
    assert_eq!(facts.items[0].id, "smallsvc::a::f");
    assert_eq!(facts.repo.commit, "c1");
    assert_eq!(facts.crates.len(), 1);
    assert_eq!(store.facts(&TreeKey::commit("unknown")).unwrap(), None);

    // A rebased commit whose files keep their keys needs nothing recomputed (ADR 0002).
    let c2 = TreeKey::commit("c2-rebased");
    store
        .put_tree(&c2, None, &files, &head("c1"), &[], &[])
        .unwrap();
    assert!(store.missing_file_facts(&c2).unwrap().is_empty());
    assert_eq!(store.facts(&c2).unwrap().unwrap(), facts);

    // The same bytes in a package that changed elsewhere are another delta.
    let c3 = TreeKey::commit("c3");
    let moved = vec![tf(&a, "p2"), tf(&b, "p2")];
    store
        .put_tree(&c3, None, &moved, &head("c3"), &[], &[])
        .unwrap();
    assert_eq!(store.missing_file_facts(&c3).unwrap(), moved);
}

#[test]
fn two_trees_with_the_same_file_bytes_keep_their_own_deltas() {
    // `main.rs` is byte-identical in both trees, but its links resolve through a package that
    // differs: each tree reads its own delta, whatever order they were recorded in.
    let mut store = Store::in_memory().unwrap();
    let in_w1 = file_facts("src/main.rs", "same bytes", &["smallsvc::main::connect"]);
    let in_w2 = file_facts("src/main.rs", "same bytes", &["smallsvc::main::reexported"]);
    let (t1, t2) = (TreeKey::commit("w1"), TreeKey::commit("w2"));
    let (f1, f2) = (vec![tf(&in_w1, "pkg-w1")], vec![tf(&in_w2, "pkg-w2")]);
    store
        .put_tree(
            &t1,
            None,
            &f1,
            &head("w1"),
            &[],
            std::slice::from_ref(&in_w1),
        )
        .unwrap();
    store
        .put_tree(
            &t2,
            None,
            &f2,
            &head("w2"),
            &[],
            std::slice::from_ref(&in_w2),
        )
        .unwrap();
    assert_eq!(store.facts(&t1).unwrap().unwrap().items, in_w1.items);
    assert_eq!(store.facts(&t2).unwrap().unwrap().items, in_w2.items);
}

#[test]
fn a_delta_for_a_file_outside_the_tree_is_refused() {
    let mut store = Store::in_memory().unwrap();
    let a = file_facts("src/a.rs", "a", &[]);
    let stray = file_facts("src/stray.rs", "s", &[]);
    let err = store.put_tree(
        &TreeKey::commit("c"),
        None,
        &[tf(&a, "p")],
        &head("c"),
        &[],
        &[stray],
    );
    assert!(err.is_err());
    assert!(!store.has_tree(&TreeKey::commit("c")).unwrap());
}

#[test]
fn a_worktree_is_its_base_plus_the_files_that_differ() {
    let mut store = Store::in_memory().unwrap();
    let a = file_facts("src/a.rs", "a v1", &["smallsvc::a::f"]);
    let b1 = file_facts("src/b.rs", "b v1", &["smallsvc::b::g"]);
    let b2 = file_facts(
        "src/b.rs",
        "b v2 edited",
        &["smallsvc::b::g", "smallsvc::b::h"],
    );
    let base = TreeKey::commit("base");
    let base_files = vec![tf(&a, "p1"), tf(&b1, "p1")];
    store
        .put_tree(
            &base,
            None,
            &base_files,
            &head("base"),
            &[],
            &[a.clone(), b1.clone()],
        )
        .unwrap();
    assert_eq!(store.tree_files(&base).unwrap(), base_files);

    let wt_files = vec![tf(&a, "p2"), tf(&b2, "p2")];
    let state =
        ContentHash::of_worktree_state(wt_files.iter().map(|f| (f.path.as_path(), &f.file_hash)));
    let wt = TreeKey::Worktree(state.clone());
    store
        .put_tree(
            &wt,
            Some("base"),
            &wt_files,
            &head(&format!("wt:{state}")),
            &[],
            &[a.clone(), b2.clone()],
        )
        .unwrap();
    store
        .set_worktree(
            Path::new("/w/smallsvc-w1"),
            &wt,
            Some("base"),
            Some("feat/x"),
        )
        .unwrap();

    assert_eq!(
        store.changed_files(&base, &wt).unwrap(),
        vec![PathBuf::from("src/b.rs")]
    );
    assert_eq!(store.facts(&wt).unwrap().unwrap().items.len(), 3);
    assert_eq!(
        store.worktree(Path::new("/w/smallsvc-w1")).unwrap(),
        Some(WorktreeState {
            tree: wt.clone(),
            base_commit: Some("base".into()),
            branch: Some("feat/x".into()),
        })
    );
    store.remove_worktree(Path::new("/w/smallsvc-w1")).unwrap();
    assert_eq!(store.worktree(Path::new("/w/smallsvc-w1")).unwrap(), None);
    // Nobody is on that state any more: it is forgotten, the base commit is kept.
    assert!(!store.has_tree(&wt).unwrap());
    assert!(store.facts(&base).unwrap().is_some());
}

#[test]
fn a_worktree_that_moves_on_forgets_its_old_state_unless_another_worktree_is_on_it() {
    let mut store = Store::in_memory().unwrap();
    let state = |name: &str| {
        let f = file_facts("src/a.rs", name, &[]);
        let files = vec![tf(&f, name)];
        let key = TreeKey::Worktree(ContentHash::of_str(name));
        (key, files, f)
    };
    let (s0, f0, d0) = state("s0");
    let (s1, f1, d1) = state("s1");
    let (s2, f2, d2) = state("s2");
    let (w1, w2) = (Path::new("/w/one"), Path::new("/w/two"));
    for (k, f, d) in [(&s0, &f0, &d0), (&s1, &f1, &d1), (&s2, &f2, &d2)] {
        store
            .put_tree(k, None, f, &head("wt"), &[], std::slice::from_ref(d))
            .unwrap();
    }
    store.set_worktree(w1, &s0, None, None).unwrap();
    store.set_worktree(w2, &s1, None, None).unwrap();

    // w1 moves from s0 to s1: s0 is forgotten with its delta.
    store.set_worktree(w1, &s1, None, None).unwrap();
    assert!(!store.has_tree(&s0).unwrap());
    assert!(!store.has_file_facts(&f0[0].facts_key).unwrap());

    // w1 moves on to s2: w2 is still on s1, so s1 stays.
    store.set_worktree(w1, &s2, None, None).unwrap();
    assert!(store.has_tree(&s1).unwrap());
    assert!(store.facts(&s1).unwrap().is_some());

    // A commit a worktree leaves is kept.
    let c = TreeKey::commit("c");
    store
        .put_tree(&c, None, &f0, &head("c"), &[], std::slice::from_ref(&d0))
        .unwrap();
    store.set_worktree(w1, &c, Some("c"), None).unwrap();
    store.set_worktree(w1, &s2, Some("c"), None).unwrap();
    assert!(store.has_tree(&c).unwrap());
    assert_eq!(store.prune_file_facts().unwrap(), 0);
}

#[test]
fn branches_point_at_commits_and_commits_are_facts() {
    let mut store = Store::in_memory().unwrap();
    store.set_branch_head("main", "c1").unwrap();
    assert_eq!(store.branch_head("main").unwrap().as_deref(), Some("c1"));
    assert_eq!(store.branch_head("nope").unwrap(), None);

    let commit = Commit {
        hash: "c1".into(),
        author: "t <t@x>".into(),
        summary: "feat(ship): port".into(),
        trailers: vec![Trailer {
            key: "Arch-Element".into(),
            value: "0123abcd".into(),
        }],
        files: vec!["src/a.rs".into()],
        element: Some("0123abcd".into()),
    };
    store.put_commits(std::slice::from_ref(&commit)).unwrap();
    store
        .put_tree(
            &TreeKey::commit("c1"),
            None,
            &[],
            &head("c1"),
            &["c1".into()],
            &[],
        )
        .unwrap();
    assert_eq!(
        store
            .facts(&TreeKey::commit("c1"))
            .unwrap()
            .unwrap()
            .commits,
        vec![commit.clone()]
    );
    assert_eq!(store.commit("c1").unwrap(), Some(commit));
}

#[test]
fn view_cache_is_per_branch_and_remembers_its_tree() {
    let store = Store::in_memory().unwrap();
    let t = TreeKey::commit("c1");
    store
        .view_cache_put("main", "positions", "unit", &t, b"{}")
        .unwrap();
    assert_eq!(
        store.view_cache_get("main", "positions", "unit").unwrap(),
        Some((t, b"{}".to_vec()))
    );
    assert_eq!(
        store.view_cache_get("feat", "positions", "unit").unwrap(),
        None
    );
    store.view_cache_invalidate("main").unwrap();
    assert_eq!(
        store.view_cache_get("main", "positions", "unit").unwrap(),
        None
    );
}

#[test]
fn plan_drafts_live_in_the_cache_until_discarded() {
    let store = Store::in_memory().unwrap();
    let mut plan = Plan::new(SessionId::new("s1"), "ship it");
    plan.add_element("add a port", "src/ship.rs");
    store.plan_draft_put(&plan).unwrap();
    assert_eq!(store.plan_drafts().unwrap(), vec!["s1".to_string()]);
    assert_eq!(store.plan_draft("s1").unwrap(), Some(plan));
    store.plan_draft_delete("s1").unwrap();
    assert_eq!(store.plan_draft("s1").unwrap(), None);
}

#[test]
fn the_store_lives_under_arch_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let arch = tmp.path().join(".arch");
    let store = Store::open(&arch).unwrap();
    drop(store);
    assert!(arch.join("cache").join(store::STORE_FILE).is_file());
    // Reopening keeps the data.
    let store = Store::open(&arch).unwrap();
    store.set_branch_head("main", "c1").unwrap();
    drop(store);
    assert_eq!(
        Store::open(&arch)
            .unwrap()
            .branch_head("main")
            .unwrap()
            .as_deref(),
        Some("c1")
    );
}

#[test]
fn facts_that_span_files_come_from_the_tree_and_set_the_item_flags() {
    let mut store = Store::in_memory().unwrap();
    let a = file_facts("src/a.rs", "a v1", &["smallsvc::a::f"]);
    let b = file_facts("src/b.rs", "b v1", &["smallsvc::b::g"]);
    let c1 = TreeKey::commit("c1");
    let files = vec![tf(&a, "p1"), tf(&b, "p1")];
    let mut head = head("c1");
    // `f` routes to `g` and re-exports it: `g` is an entry, and both facts sit in a's body.
    head.assembled.entries.push(Entry {
        item: "smallsvc::b::g".into(),
        kind: EntryKind::Framework,
        framework: Some(Framework::Axum),
        confidence: Confidence::Guessed,
        witness: Witness::Span {
            file: "src/a.rs".into(),
            line: 1,
            col: None,
        },
    });
    let reexport = Link {
        from: "smallsvc::a::f".into(),
        to: Target::Item("smallsvc::b::g".into()),
        member: None,
        kind: LinkKind::ReExports,
        confidence: Confidence::Guessed,
        witness: Witness::Span {
            file: "src/a.rs".into(),
            line: 1,
            col: None,
        },
        flags: LinkFlags::default(),
        reason: Some("pub use".into()),
    };
    head.assembled.links.push(reexport.clone());
    store
        .put_tree(&c1, None, &files, &head, &[], &[a.clone(), b.clone()])
        .unwrap();

    let facts = store.facts(&c1).unwrap().unwrap();
    assert_eq!(facts.entries.len(), 1);
    assert_eq!(facts.links, vec![reexport]);
    let g = facts.items.iter().find(|i| i.name == "g").unwrap();
    assert!(g.flags.entry && g.reexported);
    let f = facts.items.iter().find(|i| i.name == "f").unwrap();
    assert!(!f.flags.entry && !f.reexported);
}
