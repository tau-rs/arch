//! The store: per-file deltas keyed by content hash, trees, pointers, cache, drafts
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

#[test]
fn facts_assemble_from_deltas_and_a_rebase_reuses_them_by_file_hash() {
    let mut store = Store::in_memory().unwrap();
    let a = file_facts("src/a.rs", "a v1", &["smallsvc::a::f"]);
    let b = file_facts("src/b.rs", "b v1", &["smallsvc::b::g"]);
    store.put_file_facts(&a).unwrap();

    let c1 = TreeKey::commit("c1");
    let files = vec![
        (a.path.clone(), a.file_hash.clone()),
        (b.path.clone(), b.file_hash.clone()),
    ];
    store.put_tree(&c1, None, &files, &head("c1"), &[]).unwrap();
    assert!(store.has_tree(&c1).unwrap());

    // b is not computed yet: the analyzer is told exactly that.
    assert_eq!(
        store.missing_file_facts(&c1).unwrap(),
        vec![(b.path.clone(), b.file_hash.clone())]
    );
    assert!(!store.has_file_facts(&b.file_hash).unwrap());
    store.put_file_facts(&b).unwrap();
    assert!(store.missing_file_facts(&c1).unwrap().is_empty());
    assert_eq!(store.file_facts(&b.file_hash).unwrap(), Some(b.clone()));

    let facts = store.facts(&c1).unwrap().unwrap();
    assert_eq!(facts.items.len(), 2);
    assert_eq!(facts.items[0].id, "smallsvc::a::f");
    assert_eq!(facts.repo.commit, "c1");
    assert_eq!(facts.crates.len(), 1);
    assert_eq!(store.facts(&TreeKey::commit("unknown")).unwrap(), None);

    // A rebased commit with the same file contents needs nothing recomputed (ADR 0002).
    let c2 = TreeKey::commit("c2-rebased");
    store.put_tree(&c2, None, &files, &head("c1"), &[]).unwrap();
    assert!(store.missing_file_facts(&c2).unwrap().is_empty());
    assert_eq!(store.facts(&c2).unwrap().unwrap(), facts);
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
    for f in [&a, &b1, &b2] {
        store.put_file_facts(f).unwrap();
    }
    let base = TreeKey::commit("base");
    let base_files = vec![
        (a.path.clone(), a.file_hash.clone()),
        (b1.path.clone(), b1.file_hash.clone()),
    ];
    store
        .put_tree(&base, None, &base_files, &head("base"), &[])
        .unwrap();
    assert_eq!(store.tree_files(&base).unwrap(), base_files);

    let wt_files = vec![
        (a.path.clone(), a.file_hash.clone()),
        (b2.path.clone(), b2.file_hash.clone()),
    ];
    let state = ContentHash::of_worktree_state(wt_files.iter().map(|(p, h)| (p.as_path(), h)));
    let wt = TreeKey::Worktree(state.clone());
    store
        .put_tree(
            &wt,
            Some("base"),
            &wt_files,
            &head(&format!("wt:{state}")),
            &[],
        )
        .unwrap();
    store
        .set_worktree(Path::new("/w/smallsvc-w1"), &state, "base", Some("feat/x"))
        .unwrap();

    assert_eq!(
        store.changed_files(&base, &wt).unwrap(),
        vec![PathBuf::from("src/b.rs")]
    );
    assert_eq!(store.facts(&wt).unwrap().unwrap().items.len(), 3);
    let (s, b, br) = store
        .worktree(Path::new("/w/smallsvc-w1"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (s, b.as_str(), br.as_deref()),
        (state, "base", Some("feat/x"))
    );
    store.remove_worktree(Path::new("/w/smallsvc-w1")).unwrap();
    assert_eq!(store.worktree(Path::new("/w/smallsvc-w1")).unwrap(), None);
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
    let files = vec![
        (a.path.clone(), a.file_hash.clone()),
        (b.path.clone(), b.file_hash.clone()),
    ];
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
    store.put_file_facts(&a).unwrap();
    store.put_file_facts(&b).unwrap();
    store.put_tree(&c1, None, &files, &head, &[]).unwrap();

    let facts = store.facts(&c1).unwrap().unwrap();
    assert_eq!(facts.entries.len(), 1);
    assert_eq!(facts.links, vec![reexport]);
    let g = facts.items.iter().find(|i| i.name == "g").unwrap();
    assert!(g.flags.entry && g.reexported);
    let f = facts.items.iter().find(|i| i.name == "f").unwrap();
    assert!(!f.flags.entry && !f.reexported);
}
