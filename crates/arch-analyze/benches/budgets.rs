//! Budgets as benchmarks with thresholds (ADR 0026): the run fails when one is crossed.
//!
//! | benchmark | budget | what is measured |
//! |---|---|---|
//! | `first_index_cold` | < 5 s | no cache; `target/` already built by a prior `cargo check`; from opening the analyzer to the first complete facts at `resolved` confidence |
//! | `recompute_body_only` | < 500 ms | one source file saved, function bodies only: that file is re-analysed, the others carried forward (ADR 0002); from the save on disk, through the watcher's debounce, to the new facts |
//! | `recompute_declaration` | < 500 ms | one source file saved with a new declaration: the unit is re-analysed; same span |
//! | `recompute_new_file` | < 500 ms | a new module file and the `mod` line reaching it; same span |
//!
//! Runs on a copy of `repos/smallsvc` from the fixtures pin (`scripts/fetch-fixtures.sh`), with
//! `CARGO_TARGET_DIR` pointing at a directory this benchmark builds once, outside the timing.
//!
//! ```text
//! cargo bench -p arch-analyze
//! ARCH_BUDGET_SCALE=2 cargo bench -p arch-analyze   # a machine twice as slow as the reference
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use arch_analyze::watch::{Debounce, Watcher};
use arch_analyze::{Analyzer, Commits, Depth, Options, Recompute};
use arch_facts::{Confidence, Facts, Store, Target};

const FIRST_INDEX_COLD: Duration = Duration::from_secs(5);
const RECOMPUTE_ONE_FILE: Duration = Duration::from_millis(500);

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

/// Write `files`, wait for the watcher's batch and apply it; the time from the first write to the
/// new facts, once the analyzer recomputed as `expect` says and `seen` holds of the facts.
fn save_and_apply(
    repo: &Path,
    watcher: &Watcher,
    analyzer: &mut Analyzer,
    store: &mut Store,
    files: &[(&Path, String)],
    expect: Recompute,
    seen: &dyn Fn(&Facts) -> bool,
) -> Result<Duration, String> {
    let started = Instant::now();
    for (file, text) in files {
        std::fs::write(repo.join(file), text).unwrap();
    }
    let batch = watcher
        .recv_timeout(Duration::from_secs(10))
        .ok_or("the watcher reported no batch for the save")?;
    let events = analyzer.apply(&batch, store).unwrap();
    let took = started.elapsed();
    let Some(arch_facts::Event::FactsUpdated { tree: key, .. }) = events.last() else {
        return Err(format!("the batch gave no FactsUpdated: {events:?}"));
    };
    if let Some((file, _)) = files
        .iter()
        .find(|(f, _)| !batch.changes.iter().any(|c| c.path == *f))
    {
        return Err(format!("the batch misses {}: {batch:?}", file.display()));
    }
    if analyzer.last_recompute() != Some(&expect) {
        return Err(format!(
            "recomputed as {:?}, not {expect:?}",
            analyzer.last_recompute()
        ));
    }
    let facts = store.facts(key).unwrap().unwrap();
    if !seen(&facts) {
        return Err("the saved change is missing from the facts".into());
    }
    Ok(took)
}

/// The function `name` is in the facts with a resolved link out of it.
fn new_fn(name: &'static str) -> impl Fn(&Facts) -> bool {
    move |facts: &Facts| {
        let suffix = format!("{name}#fn");
        facts.items.iter().any(|i| i.name == name)
            && facts
                .links
                .iter()
                .any(|l| l.from.ends_with(&suffix) && l.confidence == Confidence::Resolved)
    }
}

fn main() -> ExitCode {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("../../fixtures/arch-fixtures/repos/smallsvc");
    if !fixture.join("Cargo.toml").is_file() {
        eprintln!(
            "{} is missing: run scripts/fetch-fixtures.sh",
            fixture.display()
        );
        return ExitCode::FAILURE;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("smallsvc");
    copy_dir(&fixture, &repo);
    // The budget assumes a built `target/` (ADR 0026): build it once, outside the timing, where
    // every later run finds it again.
    let target: PathBuf = manifest
        .join("../../target/bench-smallsvc")
        .components()
        .collect();
    let check = Command::new("cargo")
        .args(["check", "--quiet"])
        .current_dir(&repo)
        .env("CARGO_TARGET_DIR", &target)
        .status();
    if !check.is_ok_and(|s| s.success()) {
        eprintln!("cargo check of the fixture failed");
        return ExitCode::FAILURE;
    }
    let options = Options {
        depth: Depth::Resolved,
        repo_name: Some("smallsvc".into()),
        commit: None,
        commits: Commits::None,
        target_dir: Some(target),
    };

    let started = Instant::now();
    let mut analyzer = Analyzer::open(&repo, options).unwrap();
    let mut store = Store::in_memory().unwrap();
    let key = analyzer.index(&mut store).unwrap();
    let first = started.elapsed();
    let facts = store.facts(&key).unwrap().unwrap();
    let resolved = facts
        .links
        .iter()
        .filter(|l| l.confidence == Confidence::Resolved)
        .count();
    if analyzer.degraded().is_some() || resolved == 0 {
        eprintln!(
            "first index did not reach resolved confidence: {:?}",
            analyzer.degraded()
        );
        return ExitCode::FAILURE;
    }

    // Watch before the save, outside the timing, and let the backend settle.
    let watcher = Watcher::new(std::slice::from_ref(&repo), Debounce::default()).unwrap();
    while watcher.recv_timeout(Duration::from_millis(300)).is_some() {}
    // A body: `PayOrder::run` calls `Money::times` now.
    let pay = Path::new("src/app/pay.rs");
    let text = std::fs::read_to_string(repo.join(pay)).unwrap().replacen(
        "        order.mark_paid()?;\n",
        "        order.mark_paid()?;\n        let _twice = amount.times(2);\n",
        1,
    );
    let calls_times = |facts: &Facts| {
        facts.links.iter().any(|l| {
            l.from.ends_with("PayOrder::run#fn")
                && matches!(&l.to, Target::Item(t) if t.ends_with("::times#fn"))
                && l.confidence == Confidence::Resolved
        })
    };
    let body_only = match save_and_apply(
        &repo,
        &watcher,
        &mut analyzer,
        &mut store,
        &[(pay, text)],
        Recompute::Files(vec![pay.to_path_buf()]),
        &calls_times,
    ) {
        Ok(took) => took,
        Err(e) => {
            eprintln!("recompute_body_only: {e}");
            return ExitCode::FAILURE;
        }
    };
    // A declaration: a new function.
    let mut text = std::fs::read_to_string(repo.join(pay)).unwrap();
    text.push_str("\npub fn added_by_the_benchmark(order: &crate::domain::Order) -> usize { order.lines.len() }\n");
    let declaration = match save_and_apply(
        &repo,
        &watcher,
        &mut analyzer,
        &mut store,
        &[(pay, text)],
        Recompute::Unit,
        &new_fn("added_by_the_benchmark"),
    ) {
        Ok(took) => took,
        Err(e) => {
            eprintln!("recompute_declaration: {e}");
            return ExitCode::FAILURE;
        }
    };
    // A new module: a file rust-analyzer did not load, and the `mod` line that reaches it.
    let module = Path::new("src/app/mod.rs");
    let mut text = std::fs::read_to_string(repo.join(module)).unwrap();
    text.push_str("\npub mod refund;\n");
    let refund = "pub fn refund_by_the_benchmark(order: &crate::domain::Order) -> usize { order.lines.len() }\n";
    let new_file = match save_and_apply(
        &repo,
        &watcher,
        &mut analyzer,
        &mut store,
        &[
            (Path::new("src/app/refund.rs"), refund.into()),
            (module, text),
        ],
        Recompute::Unit,
        &new_fn("refund_by_the_benchmark"),
    ) {
        Ok(took) => took,
        Err(e) => {
            eprintln!("recompute_new_file: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The numbers hold on the reference machine (ADR 0026). A slower machine, such as a shared
    // CI runner, states its factor instead of pretending to be it.
    let scale: f64 = std::env::var("ARCH_BUDGET_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    if scale != 1.0 {
        println!("budgets scaled by {scale} (ARCH_BUDGET_SCALE)");
    }
    let mut ok = true;
    for (name, took, budget) in [
        ("first_index_cold", first, FIRST_INDEX_COLD),
        ("recompute_body_only", body_only, RECOMPUTE_ONE_FILE),
        ("recompute_declaration", declaration, RECOMPUTE_ONE_FILE),
        ("recompute_new_file", new_file, RECOMPUTE_ONE_FILE),
    ] {
        let budget = budget.mul_f64(scale);
        let verdict = if took < budget { "ok" } else { "OVER BUDGET" };
        println!(
            "{name:<22} {:>8.0} ms   budget {:>5} ms   {verdict}",
            took.as_secs_f64() * 1000.0,
            budget.as_millis()
        );
        ok &= took < budget;
    }
    println!(
        "items {}  links {} ({resolved} resolved, first index)",
        facts.items.len(),
        facts.links.len()
    );
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
