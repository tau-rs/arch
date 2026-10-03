//! Facts that span files are derived when the tree is assembled, not stored in a file's delta
//! (ADR 0002): a file's delta does not change when only another file's body does.

use std::path::{Path, PathBuf};

use arch_analyze::{Commits, Options, index};
use arch_facts::*;

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

fn smallsvc_copy() -> (tempfile::TempDir, PathBuf) {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        fixture.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        fixture.display()
    );
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("smallsvc");
    copy_dir(&fixture, &repo);
    (tmp, repo)
}

fn options() -> Options {
    Options {
        repo_name: Some("smallsvc".into()),
        commits: Commits::None,
        ..Default::default()
    }
}

fn delta(store: &Store, key: &TreeKey, path: impl AsRef<Path>) -> FileFacts {
    let (_, hash) = store
        .tree_files(key)
        .unwrap()
        .into_iter()
        .find(|(p, _)| p == path.as_ref())
        .unwrap();
    store.file_facts(&hash).unwrap().unwrap()
}

fn is_entry(facts: &Facts, id: &str) -> bool {
    let item = facts.items.iter().find(|i| i.id.ends_with(id)).unwrap();
    item.flags.entry && facts.entries.iter().any(|e| e.item == item.id)
}

#[test]
fn a_route_removed_in_one_file_leaves_the_handlers_delta_unchanged() {
    let (_tmp, repo) = smallsvc_copy();
    let mut store = Store::in_memory().unwrap();
    let before = index(&repo, &options(), &mut store).unwrap();
    let handlers_before = delta(&store, &before, "src/adapters/http/handlers.rs");
    assert!(is_entry(
        &store.facts(&before).unwrap().unwrap(),
        "handlers::health#fn"
    ));

    // A body-only edit in another file: the router stops routing `/health`.
    let router = repo.join("src/adapters/http/mod.rs");
    let text = std::fs::read_to_string(&router).unwrap();
    let edited = text.replace("        .route(\"/health\", get(handlers::health))\n", "");
    assert_ne!(edited, text);
    std::fs::write(&router, edited).unwrap();
    let after = index(&repo, &options(), &mut store).unwrap();
    assert_ne!(before, after);

    // The handler is no longer an entry, and the file that declares it did not change.
    let facts = store.facts(&after).unwrap().unwrap();
    assert!(!is_entry(&facts, "handlers::health#fn"));
    assert!(!facts.ports.iter().any(|p| p.name == "GET /health"));
    assert_eq!(
        delta(&store, &after, "src/adapters/http/handlers.rs"),
        handlers_before
    );
}

#[test]
fn a_delta_holds_its_files_items_and_links_and_notes_the_rest() {
    let (_tmp, repo) = smallsvc_copy();
    let mut store = Store::in_memory().unwrap();
    let key = index(&repo, &options(), &mut store).unwrap();
    let facts = store.facts(&key).unwrap().unwrap();

    // Entries, ports, externals and tables are the tree's, not a file's.
    for (path, _) in store.tree_files(&key).unwrap() {
        let d = delta(&store, &key, &path);
        assert!(d.entries.is_empty() && d.ports.is_empty(), "{path:?}");
        assert!(d.externals.is_empty() && d.tables.is_empty(), "{path:?}");
    }
    assert!(!facts.entries.is_empty() && !facts.ports.is_empty());
    assert!(!facts.externals.is_empty() && !facts.tables.is_empty());

    // The outbox's inserter queues because the dequeuer uses `SKIP LOCKED` (assembled), and the
    // table says who dequeues it.
    let outbox = facts.tables.iter().find(|t| t.name == "outbox").unwrap();
    let queue = outbox.queue.as_ref().unwrap();
    assert!(!queue.dequeuers.is_empty() && !queue.inserters.is_empty());
    for inserter in &queue.inserters {
        assert!(facts.links.iter().any(|l| &l.from == inserter
            && l.kind == LinkKind::Queues
            && l.to == Target::Table("outbox".into())));
    }
}
