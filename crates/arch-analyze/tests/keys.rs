//! A file's facts are named by its path, its content and its package id (ADR 0002): two trees
//! share a delta only when it is the same delta, and a package id is git's tree id of the
//! package plus what the analysis depends on besides.

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_analyze::package::{self, Blob, ObjectFormat};
use arch_analyze::{Commits, Options, analyze, git, index};
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

fn smallsvc() -> PathBuf {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        dir.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        dir.display()
    );
    dir
}

fn options() -> Options {
    Options {
        repo_name: Some("smallsvc".into()),
        commits: Commits::None,
        ..Default::default()
    }
}

fn main_rs(store: &Store, key: &TreeKey) -> TreeFile {
    store
        .tree_files(key)
        .unwrap()
        .into_iter()
        .find(|f| f.path == Path::new("src/main.rs"))
        .unwrap()
}

fn json(f: &Facts) -> String {
    serde_json::to_string_pretty(f).unwrap()
}

#[test]
fn two_trees_with_a_byte_identical_file_keep_their_own_facts_for_it() {
    // ADR 0002's case: `main.rs` stays byte-identical while `postgres/mod.rs` turns `connect`
    // into a re-export, so `main`'s call to `postgres::connect` lands elsewhere.
    let tmp = tempfile::tempdir().unwrap();
    let (w1, w2) = (tmp.path().join("w1"), tmp.path().join("w2"));
    copy_dir(&smallsvc(), &w1);
    copy_dir(&smallsvc(), &w2);
    let pg = w2.join("src/adapters/postgres/mod.rs");
    let text = std::fs::read_to_string(&pg).unwrap();
    let edited = text.replace(
        "pub async fn connect(",
        "pub use self::open_pool as connect;\n\npub async fn open_pool(",
    );
    assert_ne!(edited, text);
    std::fs::write(&pg, edited).unwrap();

    let cold1 = analyze(&w1, &options()).unwrap();
    let cold2 = analyze(&w2, &options()).unwrap();
    let main_links = |f: &Facts| -> Vec<Link> {
        f.links
            .iter()
            .filter(|l| l.from.ends_with("::main#fn"))
            .cloned()
            .collect()
    };
    assert_ne!(
        main_links(&cold1),
        main_links(&cold2),
        "the edit must move a link of main.rs"
    );

    // One store, both trees, in either order: each reads what a cold analysis of it gives.
    for order in [[&w1, &w2], [&w2, &w1]] {
        let mut store = Store::in_memory().unwrap();
        let keys: Vec<TreeKey> = order
            .iter()
            .map(|w| index(w, &options(), &mut store).unwrap())
            .collect();
        let (k1, k2) = if order[0] == &w1 {
            (&keys[0], &keys[1])
        } else {
            (&keys[1], &keys[0])
        };
        assert_eq!(json(&store.facts(k1).unwrap().unwrap()), json(&cold1));
        assert_eq!(json(&store.facts(k2).unwrap().unwrap()), json(&cold2));
        let (m1, m2) = (main_rs(&store, k1), main_rs(&store, k2));
        assert_eq!(m1.file_hash, m2.file_hash);
        assert_ne!(m1.facts_key, m2.facts_key);
    }
}

#[test]
fn the_same_tree_twice_reuses_every_delta() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path().join("w");
    copy_dir(&smallsvc(), &w);
    let mut store = Store::in_memory().unwrap();
    let a = index(&w, &options(), &mut store).unwrap();
    let files = store.tree_files(&a).unwrap();
    let b = index(&w, &options(), &mut store).unwrap();
    assert_eq!(a, b);
    assert_eq!(store.tree_files(&b).unwrap(), files);

    // Another unit name is another analysis: no key is shared.
    let mut other = options();
    other.repo_name = Some("renamed".into());
    let c = index(&w, &other, &mut store).unwrap();
    let renamed = store.tree_files(&c).unwrap();
    assert!(
        renamed
            .iter()
            .zip(&files)
            .all(|(x, y)| x.facts_key != y.facts_key)
    );
}

fn git_in(dir: &Path, args: &[&str], index_file: &Path) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_INDEX_FILE", index_file)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn a_package_s_tree_id_is_the_one_git_gives() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    copy_dir(&smallsvc(), &repo);
    let idx = tmp.path().join("index");
    git_in(&repo, &["init", "-q"], &idx);
    // An executable, a symlink, an ignored file and an untracked one, as the working files hold.
    let script = repo.join("scripts/run.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "#!/bin/sh\necho run\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("../Cargo.toml", repo.join("scripts/manifest")).unwrap();
    }
    std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(repo.join("ignored.txt"), "not part of the tree\n").unwrap();
    std::fs::write(repo.join("notes.txt"), "untracked, not ignored\n").unwrap();

    // What git names the working files: a fresh index, `git add -A`, `git write-tree`.
    git_in(&repo, &["add", "-A"], &idx);
    let root = git_in(&repo, &["write-tree"], &idx);
    let src = git_in(&repo, &["rev-parse", &format!("{root}:src")], &idx);

    let format = ObjectFormat::of_repo(&repo);
    let blobs: Vec<(PathBuf, Blob)> = git::files(&repo)
        .unwrap()
        .into_iter()
        .map(|f| {
            let path = repo.join(&f);
            let bytes = std::fs::read(&path).unwrap();
            let blob = Blob::of_file(format, &path, &bytes).unwrap();
            (f, blob)
        })
        .collect();
    let trees = package::tree_ids(format, &blobs);
    assert_eq!(trees[Path::new("")], root);
    assert_eq!(trees[Path::new("src")], src);
}

#[test]
fn a_save_forgets_the_state_it_leaves() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path().join("w");
    copy_dir(&smallsvc(), &w);
    let mut store = Store::in_memory().unwrap();
    let before = index(&w, &options(), &mut store).unwrap();
    let old = main_rs(&store, &before);

    let pay = w.join("src/app/pay.rs");
    let text = std::fs::read_to_string(&pay).unwrap();
    std::fs::write(&pay, format!("{text}\n// saved\n")).unwrap();
    let after = index(&w, &options(), &mut store).unwrap();

    // The worktree moved on: its old state and the deltas only it held are gone.
    assert_ne!(before, after);
    assert!(!store.has_tree(&before).unwrap());
    assert!(!store.has_file_facts(&old.facts_key).unwrap());
    assert!(
        store
            .has_file_facts(&main_rs(&store, &after).facts_key)
            .unwrap()
    );
    assert_eq!(store.prune_file_facts().unwrap(), 0);
    assert!(store.missing_file_facts(&after).unwrap().is_empty());
}
