//! `incremental_equals_cold` (ADR 0002): scripted edits on the pinned smallsvc, each followed by
//! an incremental index, give facts byte-identical to a cold analysis of the same files, in any
//! order. A save that changes function bodies only re-analyses the changed files; one that
//! changes a declaration re-analyses the unit.
//!
//! The syntax-level runs are part of `cargo test`. The resolved run loads rust-analyzer once per
//! cold analysis, so it is `#[ignore]`d and runs in release in CI's budgets job:
//!
//! ```text
//! cargo test --release -p arch-analyze --test incremental -- --ignored
//! ```

use std::path::{Path, PathBuf};

use arch_analyze::{Analyzer, Commits, Depth, Options, Recompute, analyze};
use arch_facts::Store;

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

/// What a save is expected to cost.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Expect {
    /// Bodies only, or a file that holds no Rust: the saved file alone.
    File,
    /// A declaration: the unit.
    Unit,
}

/// One scripted save: `find` becomes `replace` in `file`.
#[derive(Debug, Clone, Copy)]
struct Edit {
    name: &'static str,
    file: &'static str,
    find: &'static str,
    replace: &'static str,
    expect: Expect,
}

const PAY_BODY: Edit = Edit {
    name: "body: pay checks the total again",
    file: "src/app/pay.rs",
    find: "        order.mark_paid()?;\n",
    replace: "        order.mark_paid()?;\n        let _again = order.total()?;\n",
    expect: Expect::File,
};
const UNROUTE_HEALTH: Edit = Edit {
    name: "body: the router stops routing /health (the handler stops being an entry)",
    file: "src/adapters/http/mod.rs",
    find: "        .route(\"/health\", get(handlers::health))\n",
    replace: "        // health is not routed\n",
    expect: Expect::File,
};
const NO_SKIP_LOCKED: Edit = Edit {
    name: "body: the dequeuer stops claiming rows (the outbox stops being a queue)",
    file: "src/adapters/postgres/outbox.rs",
    find: "LIMIT $1 FOR UPDATE SKIP LOCKED)",
    replace: "LIMIT $1)",
    expect: Expect::File,
};
const CONNECT_REEXPORTED: Edit = Edit {
    name: "declaration: postgres turns `connect` into a re-export (main.rs unchanged)",
    file: "src/adapters/postgres/mod.rs",
    find: "pub async fn connect(",
    replace: "pub use self::open_pool as connect;\n\npub async fn open_pool(",
    expect: Expect::Unit,
};
const USE_ADDED: Edit = Edit {
    name: "declaration: a use",
    file: "src/app/pay.rs",
    find: "use std::sync::Arc;\n",
    replace: "use std::sync::Arc;\nuse std::fmt as _fmt;\n",
    expect: Expect::Unit,
};
const NESTED_ITEM: Edit = Edit {
    name: "declaration: an item nested in a body",
    file: "src/app/pay.rs",
    find: "        order.mark_paid()?;\n",
    replace: "        fn nested() {}\n        order.mark_paid()?;\n",
    expect: Expect::Unit,
};
const MIGRATION: Edit = Edit {
    name: "migration: a table more",
    file: "migrations/20261001000002_outbox.sql",
    find: "CREATE TABLE",
    replace: "CREATE TABLE audit (id BIGINT PRIMARY KEY);\n\nCREATE TABLE",
    expect: Expect::File,
};
const COMMENT: Edit = Edit {
    name: "body: a comment",
    file: "src/domain/order.rs",
    find: "use chrono::{DateTime, Utc};\n",
    replace: "use chrono::{DateTime, Utc}; // dates\n",
    expect: Expect::File,
};

fn options(depth: Depth) -> Options {
    Options {
        depth,
        repo_name: Some("smallsvc".into()),
        commits: Commits::None,
        ..Default::default()
    }
}

/// Apply `edit` (or undo it), index incrementally, and compare with a cold analysis.
fn step(analyzer: &mut Analyzer, store: &mut Store, repo: &Path, edit: &Edit, undo: bool) {
    let path = repo.join(edit.file);
    let text = std::fs::read_to_string(&path).unwrap();
    let (from, to) = if undo {
        (edit.replace, edit.find)
    } else {
        (edit.find, edit.replace)
    };
    assert_eq!(
        text.matches(from).count(),
        1,
        "{}: one place to edit",
        edit.name
    );
    let edited = text.replacen(from, to, 1);
    std::fs::write(&path, edited).unwrap();
    analyzer.file_changed(Path::new(edit.file)).unwrap();
    let key = analyzer.index(store).unwrap();

    let expected = match edit.expect {
        Expect::File => Recompute::Files(vec![PathBuf::from(edit.file)]),
        Expect::Unit => Recompute::Unit,
    };
    assert_eq!(
        analyzer.last_recompute(),
        Some(&expected),
        "{} (undo: {undo})",
        edit.name
    );
    let incremental = store.facts(&key).unwrap().unwrap();
    let cold = analyze(repo, &options(depth(analyzer))).unwrap();
    let (a, b) = (
        serde_json::to_string_pretty(&incremental).unwrap(),
        serde_json::to_string_pretty(&cold).unwrap(),
    );
    if a != b {
        let first = a
            .lines()
            .zip(b.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(0);
        panic!(
            "{} (undo: {undo}): incremental facts differ from cold at line {first}:\n  incremental: {}\n  cold:        {}",
            edit.name,
            a.lines().nth(first).unwrap_or(""),
            b.lines().nth(first).unwrap_or("")
        );
    }
}

fn depth(analyzer: &Analyzer) -> Depth {
    if analyzer.degraded().is_none() {
        Depth::Resolved
    } else {
        Depth::Syntax
    }
}

fn run(depth: Depth, edits: &[Edit], undo_after: bool) {
    let (_tmp, repo) = smallsvc_copy();
    let mut analyzer = Analyzer::open(&repo, options(depth)).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    assert_eq!(analyzer.last_recompute(), Some(&Recompute::Unit));
    for e in edits {
        step(&mut analyzer, &mut store, &repo, e, false);
    }
    if undo_after {
        for e in edits.iter().rev() {
            step(&mut analyzer, &mut store, &repo, e, true);
        }
    }
}

/// Every order of `items`.
fn permutations<T: Copy>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let first = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, first);
            out.push(p);
        }
    }
    out
}

#[test]
fn incremental_equals_cold_for_every_kind_of_save_and_its_undo() {
    run(
        Depth::Syntax,
        &[
            PAY_BODY,
            UNROUTE_HEALTH,
            NO_SKIP_LOCKED,
            CONNECT_REEXPORTED,
            USE_ADDED,
            NESTED_ITEM,
            MIGRATION,
            COMMENT,
        ],
        true,
    );
}

#[test]
fn incremental_equals_cold_for_the_same_edits_in_any_order() {
    let edits = [PAY_BODY, UNROUTE_HEALTH, NO_SKIP_LOCKED, CONNECT_REEXPORTED];
    for order in permutations(&edits) {
        run(Depth::Syntax, &order, false);
    }
}

#[test]
fn nothing_changed_reindexes_nothing() {
    let (_tmp, repo) = smallsvc_copy();
    let mut analyzer = Analyzer::open(&repo, options(Depth::Syntax)).unwrap();
    let mut store = Store::in_memory().unwrap();
    let a = analyzer.index(&mut store).unwrap();
    let b = analyzer.index(&mut store).unwrap();
    assert_eq!(a, b);
    assert_eq!(analyzer.last_recompute(), Some(&Recompute::Files(vec![])));
}

#[test]
#[ignore = "loads rust-analyzer once per cold analysis: run in release (CI budgets job)"]
fn incremental_equals_cold_at_resolved_depth() {
    run(
        Depth::Resolved,
        &[PAY_BODY, CONNECT_REEXPORTED, UNROUTE_HEALTH],
        false,
    );
}
