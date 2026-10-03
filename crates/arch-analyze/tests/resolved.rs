//! The rust-analyzer pass: links come from the type-checked view and are `resolved`; pattern
//! links stay `guessed`; a repository rust-analyzer cannot load falls back to syntax-level facts
//! with the reason recorded (ADR 0010); a changed file is picked up without loading again.

use std::path::{Path, PathBuf};

use arch_analyze::{Analyzer, Commits, Depth, Options, analyze};
use arch_facts::*;

fn options() -> Options {
    Options {
        depth: Depth::Resolved,
        repo_name: Some("smallsvc".into()),
        commit: Some("f".repeat(40)),
        commits: Commits::None,
        target_dir: None,
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

fn item(id: &str) -> Target {
    Target::Item(id.into())
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

#[test]
fn smallsvc_links_are_resolved_and_patterns_stay_guessed() {
    let f = analyze(&smallsvc(), &options()).unwrap();
    assert!(f.analyzer.degraded.is_empty(), "{:?}", f.analyzer.degraded);
    assert!(f.analyzer.version.contains("ra_ap_"));
    let find = |from: &str, kind: LinkKind, to: &Target| {
        f.links
            .iter()
            .find(|l| l.from.ends_with(from) && l.kind == kind && &l.to == to)
    };

    // Type-checked facts: resolved, no reason needed.
    let through_port = find(
        "app::pay::impl PayOrder::run#fn",
        LinkKind::CallsPort,
        &item("orderly::ports::payment_gateway::PaymentGateway::charge#fn"),
    )
    .expect("the use case calls the gateway port");
    assert_eq!(
        (through_port.confidence, through_port.reason.as_deref()),
        (Confidence::Resolved, None)
    );
    // Inside `#[async_trait] impl`, which only exists after macro expansion.
    assert!(
        find(
            "impl Outbox for PgOutbox::dequeue#fn",
            LinkKind::Constructs,
            &item("orderly::ports::outbox::OutboxMessage#struct")
        )
        .is_some_and(|l| l.confidence == Confidence::Resolved)
    );
    // Inside `tokio::select!`.
    assert!(
        find(
            "worker::run_outbox_worker#fn",
            LinkKind::Calls,
            &item("orderly::app::notify::impl NotifyCustomer::drain#fn")
        )
        .is_some()
    );
    // Only type inference knows which `default` this is.
    assert!(
        find(
            "main#fn",
            LinkKind::Calls,
            &item("orderly::worker::impl Default for WorkerConfig::default#fn")
        )
        .is_some()
    );
    // The allowed violation, now a resolved fact.
    assert!(
        find(
            "NotifyCustomer::deliver#fn",
            LinkKind::Constructs,
            &item("orderly::adapters::email::LogNotifier#struct")
        )
        .is_some_and(|l| l.confidence == Confidence::Resolved)
    );
    assert!(
        find(
            "impl OrderRepository for PgOrderRepository#impl",
            LinkKind::Implements,
            &item("orderly::ports::order_repo::OrderRepository#trait")
        )
        .is_some()
    );

    // Patterns: still guesses, with their reason.
    for kind in [
        LinkKind::Routes,
        LinkKind::HandsOff,
        LinkKind::Queues,
        LinkKind::Wires,
    ] {
        let links: Vec<&Link> = f.links.iter().filter(|l| l.kind == kind).collect();
        assert!(!links.is_empty(), "{kind:?}");
        assert!(
            links
                .iter()
                .all(|l| l.confidence == Confidence::Guessed && l.reason.is_some()),
            "{kind:?}"
        );
    }
    assert!(
        f.links
            .iter()
            .filter(|l| matches!(l.to, Target::Table(_)))
            .all(|l| l.confidence == Confidence::Guessed)
    );
    // Every link has one of the two confidences and points at something that exists.
    let items: std::collections::BTreeSet<&str> = f.items.iter().map(|i| i.id.as_str()).collect();
    let externals: std::collections::BTreeSet<&str> =
        f.externals.iter().map(|e| e.id.as_str()).collect();
    for l in &f.links {
        assert_eq!(
            l.reason.is_some(),
            l.confidence == Confidence::Guessed,
            "{l:?}"
        );
        match &l.to {
            Target::Item(id) => assert!(items.contains(id.as_str()), "{l:?}"),
            Target::External(id) => assert!(externals.contains(id.as_str()), "{l:?}"),
            _ => {}
        }
    }
    // The spawned worker is still an entry and still not a direct call.
    let entry = |id: &str| {
        f.entries
            .iter()
            .find(|e| e.item == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    let worker = entry("orderly::worker::run_outbox_worker#fn");
    // ADR 0028: the spawn is in `main`'s own body, so the entry is resolved.
    assert_eq!(
        (worker.kind, worker.confidence),
        (EntryKind::SpawnedWorker, Confidence::Resolved)
    );
    let main = entry("orderly[bin:orderly]::main#fn");
    assert_eq!(
        (main.kind, main.confidence),
        (EntryKind::Main, Confidence::Resolved)
    );
    let handler = entry("orderly::adapters::http::handlers::pay_order#fn");
    assert_eq!(
        (handler.kind, handler.confidence),
        (EntryKind::Framework, Confidence::Guessed)
    );
    assert_eq!(
        f.entries.len(),
        7,
        "main, five handlers, the worker; tests are not entries"
    );
    assert!(
        find(
            "main#fn",
            LinkKind::Calls,
            &item("orderly::worker::run_outbox_worker#fn")
        )
        .is_none()
    );
    // A crate reached only through another one is witnessed by the lock file.
    let core = f
        .externals
        .iter()
        .find(|e| e.id == "external:crate:sqlx-core")
        .expect("sqlx-core");
    assert!(matches!(&core.witness, Witness::Declared { file, .. } if file == "Cargo.lock"));

    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/facts.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    let json = serde_json::to_value(&f).unwrap();
    let errors: Vec<String> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&json)
        .map(|e| e.to_string())
        .take(3)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_repository_rust_analyzer_cannot_load_degrades_with_the_reason() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"broken\"\nversion = \"0.1.0\"\nedition = \"2077\"\n",
            ),
            ("src/main.rs", "fn helper() {}\nfn main() { helper() }\n"),
        ],
    );
    let f = analyze(tmp.path(), &options()).unwrap();
    assert_eq!(f.analyzer.degraded.len(), 1);
    let reason = &f.analyzer.degraded[0].reason;
    assert!(
        reason.starts_with("rust-analyzer could not load the repository:"),
        "{reason}"
    );
    assert!(
        !f.links.is_empty() && f.links.iter().all(|l| l.confidence == Confidence::Guessed),
        "all facts guessed"
    );
}

#[test]
fn a_changed_file_is_recomputed_without_loading_again() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            ("src/main.rs", "mod a;\nmod b;\nfn main() { a::run() }\n"),
            ("src/a.rs", "pub fn run() {}\n"),
            (
                "src/b.rs",
                "pub struct Meter;\nimpl Meter { pub fn tick(&self) {} }\npub fn make() -> Meter { Meter }\n",
            ),
        ],
    );
    // No commit override: each state of the directory is keyed by its own worktree hash.
    let mut analyzer = Analyzer::open(
        tmp.path(),
        Options {
            commit: None,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(analyzer.degraded(), None);
    let mut store = Store::in_memory().unwrap();
    let before = analyzer.index(&mut store).unwrap();
    let facts = store.facts(&before).unwrap().unwrap();
    let tick = item("live[bin:live]::b::impl Meter::tick#fn");
    assert!(!facts.links.iter().any(|l| l.to == tick));

    // The receiver's type is only known by inference: `make()` returns a `Meter`.
    std::fs::write(
        tmp.path().join("src/a.rs"),
        "pub fn run() { let m = crate::b::make(); m.tick() }\n",
    )
    .unwrap();
    analyzer.file_changed(Path::new("src/a.rs")).unwrap();
    let started = std::time::Instant::now();
    let after = analyzer.index(&mut store).unwrap();
    let took = started.elapsed();
    let facts = store.facts(&after).unwrap().unwrap();
    let link = facts
        .links
        .iter()
        .find(|l| l.to == tick)
        .expect("the new call is seen");
    assert_eq!(
        (link.from.as_str(), link.kind, link.confidence),
        (
            "live[bin:live]::a::run#fn",
            LinkKind::Calls,
            Confidence::Resolved
        )
    );
    assert_eq!(
        store.changed_files(&before, &after).unwrap(),
        [PathBuf::from("src/a.rs")]
    );
    assert!(
        took.as_secs() < 5,
        "recompute took {took:?}; the budget itself is checked by the benchmark"
    );
}

/// A crate whose `src/lib.rs` defines `one`, opened at resolved depth and indexed once.
fn opened_lib(root: &Path) -> (Analyzer, Store) {
    write(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            ("src/lib.rs", "pub fn one() {}\n"),
        ],
    );
    let mut analyzer = Analyzer::open(
        root,
        Options {
            repo_name: Some("demo".into()),
            commit: None,
            target_dir: Some(root.join("target")),
            ..options()
        },
    )
    .unwrap();
    assert_eq!(analyzer.degraded(), None);
    let mut store = Store::in_memory().unwrap();
    analyzer.index(&mut store).unwrap();
    (analyzer, store)
}

/// After `changed` are told to the analyzer and it re-indexes: the calls both ways between
/// `one` and `two` (at `module`) are resolved, and nothing is degraded.
fn assert_new_module_resolved(analyzer: &mut Analyzer, store: &mut Store, module: &str) {
    let key = analyzer.index(store).unwrap();
    let facts = store.facts(&key).unwrap().unwrap();
    let call = |from: &str, to: &str| {
        facts
            .links
            .iter()
            .find(|l| l.from == from && l.kind == LinkKind::Calls && l.to == item(to))
            .map(|l| l.confidence)
    };
    let two = format!("demo::{module}::two#fn");
    assert_eq!(
        (call(&two, "demo::one#fn"), call("demo::one#fn", &two)),
        (Some(Confidence::Resolved), Some(Confidence::Resolved)),
        "{:#?}",
        facts.links
    );
    assert_eq!(analyzer.degraded(), None);
    assert!(
        facts.analyzer.degraded.is_empty(),
        "{:?}",
        facts.analyzer.degraded
    );
}

#[test]
fn a_file_created_after_load_is_resolved_like_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut analyzer, mut store) = opened_lib(tmp.path());

    write(
        tmp.path(),
        &[
            ("src/b.rs", "pub fn two() { crate::one() }\n"),
            ("src/lib.rs", "pub mod b;\npub fn one() { b::two() }\n"),
        ],
    );
    analyzer.file_changed(Path::new("src/b.rs")).unwrap();
    analyzer.file_changed(Path::new("src/lib.rs")).unwrap();
    assert_new_module_resolved(&mut analyzer, &mut store, "b");
}

#[test]
fn a_file_created_in_a_new_directory_after_load_is_resolved_like_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut analyzer, mut store) = opened_lib(tmp.path());

    write(
        tmp.path(),
        &[
            ("src/x/mod.rs", "pub fn two() { crate::one() }\n"),
            ("src/lib.rs", "pub mod x;\npub fn one() { x::two() }\n"),
        ],
    );
    analyzer.file_changed(Path::new("src/lib.rs")).unwrap();
    analyzer.file_changed(Path::new("src/x/mod.rs")).unwrap();
    assert_new_module_resolved(&mut analyzer, &mut store, "x");
}

#[test]
fn a_file_rust_analyzer_cannot_take_keeps_guessed_links_and_degrades_its_crate() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut analyzer, mut store) = opened_lib(tmp.path());

    // A new member is a new crate: rust-analyzer only sees it after loading again.
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"sub\"]\n\n[dependencies]\nsub = { path = \"sub\" }\n",
            ),
            (
                "sub/Cargo.toml",
                "[package]\nname = \"sub\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            (
                "sub/src/lib.rs",
                "pub fn three() { four() }\nfn four() {}\n",
            ),
            ("src/lib.rs", "pub fn one() { sub::three() }\n"),
        ],
    );
    for f in [
        "Cargo.toml",
        "sub/Cargo.toml",
        "sub/src/lib.rs",
        "src/lib.rs",
    ] {
        analyzer.file_changed(Path::new(f)).unwrap();
    }
    let key = analyzer.index(&mut store).unwrap();
    let facts = store.facts(&key).unwrap().unwrap();
    let link = facts
        .links
        .iter()
        .find(|l| l.from == "sub::three#fn" && l.to == item("sub::four#fn"));
    assert_eq!(
        link.map(|l| l.confidence),
        Some(Confidence::Guessed),
        "{:#?} {:#?}",
        facts.items.iter().map(|i| &i.id).collect::<Vec<_>>(),
        facts.crates
    );
    let reasons: Vec<(&str, &str)> = facts
        .analyzer
        .degraded
        .iter()
        .map(|d| (d.crate_name.as_str(), d.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        [(
            "sub",
            "rust-analyzer has no module for sub/src/lib.rs: links guessed until the analyzer is opened again"
        )]
    );
}
