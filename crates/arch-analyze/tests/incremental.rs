//! `incremental_equals_cold` (ADR 0002): scripted edits on the pinned smallsvc, each followed by
//! an incremental index, give facts byte-identical to a cold analysis of the same files, in any
//! order. A save that changes function bodies only re-analyses the changed files; one that
//! changes a declaration re-analyses its package and the unit packages that depend on it, on
//! smallsvc (one package) and on a four-package workspace written here.
//!
//! The syntax-level runs are part of `cargo test`. The resolved run loads rust-analyzer once per
//! cold analysis, so it is `#[ignore]`d and runs in release in CI's budgets job:
//!
//! ```text
//! cargo test --release -p arch-analyze --test incremental -- --ignored
//! ```

use std::path::{Path, PathBuf};

use arch_analyze::watch::{Batch, Change};
use arch_analyze::{Analyzer, Commits, Depth, Options, Recompute, analyze};
use arch_facts::{Attribution, Store};

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
    /// A declaration: these packages, the saved file's and those depending on it.
    Packages(&'static [&'static str]),
    /// A manifest: the unit.
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
    expect: Expect::Packages(&["orderly"]),
};
const USE_ADDED: Edit = Edit {
    name: "declaration: a use",
    file: "src/app/pay.rs",
    find: "use std::sync::Arc;\n",
    replace: "use std::sync::Arc;\nuse std::fmt as _fmt;\n",
    expect: Expect::Packages(&["orderly"]),
};
const NESTED_ITEM: Edit = Edit {
    name: "declaration: an item nested in a body",
    file: "src/app/pay.rs",
    find: "        order.mark_paid()?;\n",
    replace: "        fn nested() {}\n        order.mark_paid()?;\n",
    expect: Expect::Packages(&["orderly"]),
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

const MANIFEST: Edit = Edit {
    name: "manifest: Cargo.toml saved (rust-analyzer loads again at resolved depth)",
    file: "Cargo.toml",
    find: "publish = false\n",
    replace: "publish = false\nrust-version = \"1.80\"\n",
    expect: Expect::Unit,
};

fn options(depth: Depth, name: &str) -> Options {
    Options {
        depth,
        repo_name: Some(name.into()),
        commits: Commits::None,
        ..Default::default()
    }
}

/// Apply `edit` (or undo it), index incrementally, and compare with a cold analysis.
fn step(analyzer: &mut Analyzer, store: &mut Store, repo: &Path, edit: &Edit, undo: bool) {
    let name = repo.file_name().unwrap().to_str().unwrap();
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
        Expect::Packages(p) => Recompute::Packages(p.iter().map(|p| p.to_string()).collect()),
        Expect::Unit => Recompute::Unit,
    };
    assert_eq!(
        analyzer.last_recompute(),
        Some(&expected),
        "{} (undo: {undo})",
        edit.name
    );
    let incremental = store.facts(&key).unwrap().unwrap();
    let cold = analyze(repo, &options(depth(analyzer), name)).unwrap();
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
    run_on(&repo, depth, edits, undo_after);
}

fn run_on(repo: &Path, depth: Depth, edits: &[Edit], undo_after: bool) {
    let name = repo.file_name().unwrap().to_str().unwrap();
    let mut analyzer = Analyzer::open(repo, options(depth, name)).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    assert_eq!(analyzer.last_recompute(), Some(&Recompute::Unit));
    for e in edits {
        step(&mut analyzer, &mut store, repo, e, false);
    }
    if undo_after {
        for e in edits.iter().rev() {
            step(&mut analyzer, &mut store, repo, e, true);
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
            MANIFEST,
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
    let mut analyzer = Analyzer::open(&repo, options(Depth::Syntax, "smallsvc")).unwrap();
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
        &[PAY_BODY, MANIFEST, CONNECT_REEXPORTED, UNROUTE_HEALTH],
        false,
    );
}

/// A workspace of four packages: `app` (the bin) uses `leaf` and `side`, which both use `base`;
/// `leaf`'s tests use `side` (a dev-dependency).
///
/// ```text
/// app ──► leaf ──► base
///  │       ┆ dev   ▲
///  └────► side ───┘
/// ```
fn workspace() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("shop");
    let manifest = |name: &str, deps: &[&str], dev: &[&str]| {
        let mut m = format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
        );
        for d in deps {
            m.push_str(&format!("{d} = {{ path = \"../{d}\" }}\n"));
        }
        if !dev.is_empty() {
            m.push_str("\n[dev-dependencies]\n");
        }
        for d in dev {
            m.push_str(&format!("{d} = {{ path = \"../{d}\" }}\n"));
        }
        m
    };
    let files = [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\", \"base\", \"leaf\", \"side\"]\nresolver = \"2\"\n"
                .to_string(),
        ),
        ("app/Cargo.toml", manifest("app", &["base", "leaf", "side"], &[])),
        (
            "app/src/main.rs",
            "fn main() {\n    let n = leaf::leaf_fn() + side::side_fn();\n    base::Thing::new().size(n);\n}\n".into(),
        ),
        ("base/Cargo.toml", manifest("base", &[], &[])),
        (
            "base/src/lib.rs",
            "pub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Thing {\n        Thing\n    }\n\n    pub fn size(&self, n: u32) -> u32 {\n        n\n    }\n}\n\npub fn base_fn() -> u32 {\n    1\n}\n\npub fn use_thing() {\n    let t = Thing::new();\n    t.ping();\n}\n".into(),
        ),
        ("leaf/Cargo.toml", manifest("leaf", &["base"], &["side"])),
        (
            "leaf/src/lib.rs",
            "pub fn leaf_fn() -> u32 {\n    base::base_fn()\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn sums() {\n        assert_eq!(super::leaf_fn() + side::side_fn(), 3);\n    }\n}\n".into(),
        ),
        ("side/Cargo.toml", manifest("side", &["base"], &[])),
        (
            "side/src/lib.rs",
            // `side` names `leaf` without depending on it, as code being edited can.
            "pub fn side_fn() -> u32 {\n    base::base_fn() + 1\n}\n\npub fn not_a_dependency() -> u32 {\n    leaf::leaf_more()\n}\n".into(),
        ),
    ];
    for (path, text) in files {
        let p = repo.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    (tmp, repo)
}

const LEAF_FN: Edit = Edit {
    name: "declaration in a package only the bin depends on",
    file: "leaf/src/lib.rs",
    find: "pub fn leaf_fn() -> u32 {\n",
    replace: "pub fn leaf_more() -> u32 {\n    2\n}\n\npub fn leaf_fn() -> u32 {\n",
    expect: Expect::Packages(&["app", "leaf"]),
};
const LEAF_IMPL: Edit = Edit {
    name: "an impl in `leaf` for a `base` type: `base` cannot see it, its facts are carried",
    file: "leaf/src/lib.rs",
    find: "pub fn leaf_fn() -> u32 {\n",
    replace: "pub trait Ping {\n    fn ping(&self);\n}\n\nimpl Ping for base::Thing {\n    fn ping(&self) {}\n}\n\npub fn leaf_fn() -> u32 {\n",
    expect: Expect::Packages(&["app", "leaf"]),
};
const BASE_FN: Edit = Edit {
    name: "declaration in the package every other one depends on",
    file: "base/src/lib.rs",
    find: "pub fn base_fn() -> u32 {\n    1\n",
    replace: "pub fn base_fn() -> u32 {\n    base_two()\n}\n\npub fn base_two() -> u32 {\n    2\n",
    expect: Expect::Packages(&["app", "base", "leaf", "side"]),
};
const SIDE_FN: Edit = Edit {
    name: "declaration in a package whose only dependent besides the bin is `leaf`'s tests",
    file: "side/src/lib.rs",
    find: "pub fn side_fn() -> u32 {\n",
    replace: "pub fn side_more() -> u32 {\n    3\n}\n\npub fn side_fn() -> u32 {\n",
    expect: Expect::Packages(&["app", "leaf", "side"]),
};
const SIDE_BODY: Edit = Edit {
    name: "body in a package nothing but the bin depends on",
    file: "side/src/lib.rs",
    find: "base::base_fn() + 1",
    replace: "base::base_fn() + 2",
    expect: Expect::File,
};
const APP_FN: Edit = Edit {
    name: "declaration in the bin",
    file: "app/src/main.rs",
    find: "fn main() {\n",
    replace: "fn helper() -> u32 {\n    side::side_fn()\n}\n\nfn main() {\n",
    expect: Expect::Packages(&["app"]),
};

#[test]
fn a_crate_sees_only_the_packages_it_depends_on() {
    let (_tmp, repo) = workspace();
    for e in [LEAF_FN, LEAF_IMPL] {
        let path = repo.join(e.file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen(e.find, e.replace, 1)).unwrap();
    }
    let facts = analyze(&repo, &options(Depth::Syntax, "shop")).unwrap();
    let from = |item: &str| {
        facts
            .links
            .iter()
            .filter(|l| l.from == item)
            .map(|l| format!("{:?} {:?}", l.kind, l.to))
            .collect::<Vec<_>>()
    };
    // `base` does not depend on `leaf`: `leaf`'s impl of `Ping` is not what `t.ping()` calls.
    let base = from("base::use_thing#fn");
    assert!(!base.iter().any(|l| l.contains("leaf")), "{base:?}");
    // `side` does not depend on `leaf`: `leaf::leaf_more` is not a unit item to it.
    let side = from("side::not_a_dependency#fn");
    assert!(!side.iter().any(|l| l.contains("leaf")), "{side:?}");
    // `leaf`'s tests use `side`, a dev-dependency.
    let test = from("leaf::tests::sums#fn");
    assert!(
        test.iter().any(|l| l.contains("side::side_fn#fn")),
        "{test:?}"
    );
    // `app` depends on both and sees them.
    let app = from("app[bin:app]::main#fn");
    assert!(
        app.iter().any(|l| l.contains("leaf::leaf_fn#fn")),
        "{app:?}"
    );
}

#[test]
fn incremental_equals_cold_across_packages() {
    let (_tmp, repo) = workspace();
    run_on(
        &repo,
        Depth::Syntax,
        &[LEAF_FN, LEAF_IMPL, BASE_FN, SIDE_FN, SIDE_BODY, APP_FN],
        true,
    );
}

#[test]
fn incremental_equals_cold_across_packages_in_any_order() {
    for order in permutations(&[LEAF_IMPL, BASE_FN, SIDE_BODY, APP_FN]) {
        let (_tmp, repo) = workspace();
        run_on(&repo, Depth::Syntax, &order, true);
    }
}

#[test]
#[ignore = "loads rust-analyzer once per cold analysis: run in release (CI budgets job)"]
fn incremental_equals_cold_across_packages_at_resolved_depth() {
    let (_tmp, repo) = workspace();
    run_on(&repo, Depth::Resolved, &[LEAF_IMPL, BASE_FN, APP_FN], true);
}

#[test]
fn a_batch_reads_again_only_the_files_it_names() {
    let (_tmp, repo) = workspace();
    let mut analyzer = Analyzer::open(&repo, options(Depth::Syntax, "shop")).unwrap();
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    let edit = |e: &Edit| {
        let path = repo.join(e.file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen(e.find, e.replace, 1)).unwrap();
    };
    // Two saves; the batch names one of them.
    edit(&SIDE_BODY);
    edit(&Edit {
        file: "leaf/src/lib.rs",
        find: "base::base_fn()\n",
        replace: "base::base_fn() + 3\n",
        ..SIDE_BODY
    });
    let batch = Batch {
        worktree: repo.canonicalize().unwrap(),
        changes: vec![Change {
            path: SIDE_BODY.file.into(),
            removed: false,
            attribution: Attribution::You,
        }],
    };
    analyzer.apply(&batch, &mut store).unwrap();
    assert_eq!(
        analyzer.last_recompute(),
        Some(&Recompute::Files(vec![PathBuf::from(SIDE_BODY.file)]))
    );
    // An index without a batch reads every file and finds the other one.
    analyzer.index(&mut store).unwrap();
    assert_eq!(
        analyzer.last_recompute(),
        Some(&Recompute::Files(vec![PathBuf::from("leaf/src/lib.rs")]))
    );
}
