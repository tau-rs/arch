//! Golden round trip: a published `facts.json` split into per-file deltas, stored, and
//! reassembled by the store is byte-identical to the original (ADR 0002; stability rules in
//! `docs/arch-facts.md`).
//!
//! Runs on `schemas/examples/facts.minimal.json` always, and on every
//! `fixtures/arch-fixtures/golden/<repo>/facts.json` present under the pin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use arch_facts::*;

fn witness_file(w: &Witness) -> Option<&str> {
    match w {
        Witness::Span { file, .. } | Witness::Declared { file, .. } => Some(file),
        Witness::Tool { .. } => None,
    }
}

/// Split a document into the per-file deltas the analyzer would have produced: items by their
/// file, links by their `from` item's file, the rest by their witness file.
fn split(facts: &Facts) -> (TreeHead, Vec<FileFacts>) {
    let item_file: BTreeMap<&str, &str> = facts
        .items
        .iter()
        .map(|i| (i.id.as_str(), i.file.as_str()))
        .collect();
    let mut by_file: BTreeMap<String, FileFacts> = BTreeMap::new();
    fn delta<'a>(by_file: &'a mut BTreeMap<String, FileFacts>, file: &str) -> &'a mut FileFacts {
        by_file
            .entry(file.to_string())
            .or_insert_with(|| FileFacts::empty(file, ContentHash::of_str(file)))
    }
    for i in &facts.items {
        delta(&mut by_file, &i.file).items.push(i.clone());
    }
    for l in &facts.links {
        delta(&mut by_file, item_file[l.from.as_str()])
            .links
            .push(l.clone());
    }
    for p in &facts.ports {
        delta(
            &mut by_file,
            witness_file(&p.witness).expect("port witness has a file"),
        )
        .ports
        .push(p.clone());
    }
    for e in &facts.externals {
        delta(
            &mut by_file,
            witness_file(&e.witness).expect("external witness has a file"),
        )
        .externals
        .push(e.clone());
    }
    for e in &facts.entries {
        delta(
            &mut by_file,
            witness_file(&e.witness).expect("entry witness has a file"),
        )
        .entries
        .push(e.clone());
    }
    for t in &facts.tables {
        delta(
            &mut by_file,
            witness_file(&t.witness).expect("table witness has a file"),
        )
        .tables
        .push(t.clone());
    }
    let head = TreeHead {
        repo: facts.repo.clone(),
        analyzer: facts.analyzer.clone(),
        crates: facts.crates.clone(),
        assembled: Default::default(),
    };
    (head, by_file.into_values().collect())
}

fn round_trip(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let golden: Facts =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    let (head, deltas) = split(&golden);
    let mut store = Store::in_memory().unwrap();
    let files: Vec<(PathBuf, ContentHash)> = deltas
        .iter()
        .map(|d| (d.path.clone(), d.file_hash.clone()))
        .collect();
    // Deltas arrive in reverse order: the store, not the producer, owns the order.
    for d in deltas.iter().rev() {
        store.put_file_facts(d).unwrap();
    }
    store.put_commits(&golden.commits).unwrap();
    let key = TreeKey::commit(&golden.repo.commit);
    let hashes: Vec<String> = golden.commits.iter().map(|c| c.hash.clone()).collect();
    store.put_tree(&key, None, &files, &head, &hashes).unwrap();
    assert!(store.missing_file_facts(&key).unwrap().is_empty());

    let assembled = store.facts(&key).unwrap().unwrap();
    assert_eq!(
        assembled,
        golden,
        "{}: store reassembly differs",
        path.display()
    );
    let a = serde_json::to_string_pretty(&assembled).unwrap();
    let g = serde_json::to_string_pretty(&golden).unwrap();
    assert_eq!(a, g, "{}: serialized bytes differ", path.display());
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn the_published_example_round_trips_through_the_store() {
    round_trip(&repo_root().join("schemas/examples/facts.minimal.json"));
}

/// The arch-fixtures commit pinned in `fixtures/pin.toml`, if any.
fn pinned_commit() -> Option<String> {
    let pin = std::fs::read_to_string(repo_root().join("fixtures/pin.toml")).unwrap();
    let section = pin.split("[arch-fixtures]").nth(1)?.split("\n[").next()?;
    let commit = section
        .lines()
        .find_map(|l| l.trim().strip_prefix("commit"))?;
    let commit = commit
        .trim_start_matches([' ', '='])
        .trim()
        .trim_start_matches('"');
    let commit = commit.split('"').next()?.trim();
    (!commit.is_empty()).then(|| commit.to_string())
}

/// `fixtures/arch-fixtures/`, which must be present when a commit is pinned
/// (`scripts/fetch-fixtures.sh`); `None` when nothing is pinned yet.
fn fixtures_root() -> Option<PathBuf> {
    let commit = pinned_commit()?;
    let dir = repo_root().join("fixtures/arch-fixtures");
    assert!(
        dir.join("golden").is_dir(),
        "fixtures/pin.toml pins arch-fixtures at {commit} but {} is missing: run scripts/fetch-fixtures.sh",
        dir.display()
    );
    Some(dir)
}

/// `golden/<repo>/sizes.json` as arch-fixtures publishes it; `items` and `links` are null until
/// arch-analyze fills them.
#[derive(serde::Deserialize)]
struct Sizes {
    repo: String,
    crates: u64,
    items: Option<u64>,
    links: Option<u64>,
}

#[test]
fn every_pinned_golden_repo_has_sizes_and_its_facts_round_trip_through_the_store() {
    let Some(root) = fixtures_root() else {
        eprintln!("fixtures/pin.toml pins no arch-fixtures commit yet");
        return;
    };
    let mut repos = 0;
    for entry in std::fs::read_dir(root.join("golden")).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        repos += 1;
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let sizes: Sizes =
            serde_json::from_str(&std::fs::read_to_string(dir.join("sizes.json")).unwrap())
                .unwrap_or_else(|e| panic!("{name}/sizes.json: {e}"));
        assert_eq!(sizes.repo, name);
        let facts_path = dir.join("facts.json");
        if facts_path.is_file() {
            round_trip(&facts_path);
            let facts: Facts =
                serde_json::from_str(&std::fs::read_to_string(&facts_path).unwrap()).unwrap();
            assert_eq!(
                facts.crates.len() as u64,
                sizes.crates,
                "{name}: crates in facts.json vs sizes.json"
            );
            if let Some(n) = sizes.items {
                assert_eq!(
                    facts.items.len() as u64,
                    n,
                    "{name}: items in facts.json vs sizes.json"
                );
            }
            if let Some(n) = sizes.links {
                assert_eq!(
                    facts.links.len() as u64,
                    n,
                    "{name}: links in facts.json vs sizes.json"
                );
            }
        } else {
            eprintln!("{name}: sizes.json only; facts.json arrives with arch-analyze (#9)");
        }
    }
    assert!(
        repos >= 4,
        "expected ripgrep, zero2prod, smallsvc, zed under golden/, found {repos}"
    );
}

#[test]
fn smallsvc_dot_arch_files_parse_with_the_readers() {
    let Some(root) = fixtures_root() else { return };
    let arch = ArchDir::of_repo(&root.join("repos/smallsvc"));
    assert!(arch.exists(), "smallsvc has a .arch/");

    let areas = arch.read_areas().unwrap();
    assert_eq!(areas.rule, Some(ColumnRule::Hexagon));
    assert_eq!(areas.main_bin.as_deref(), Some("orderly"));
    assert_eq!(
        areas.area("http").and_then(|a| a.side),
        Some(Column::Driving)
    );
    assert!(
        areas.areas.iter().all(|a| !a.paths.is_empty()),
        "overrides name paths"
    );

    let rules = arch.read_rules().unwrap();
    assert_eq!(
        rules.rules.len(),
        3,
        "the ADR 0006 template has three rules"
    );
    assert_eq!(rules.rules, Rules::v1_template().rules);
    assert_eq!(
        rules.lints.values().filter(|l| l.is_on()).count(),
        5,
        "five lints on"
    );

    let allows = arch.read_allows().unwrap();
    assert_eq!(allows.allows.len(), 1);
    assert!(allows.allows(
        "src/app/notify.rs::NotifyCustomer::deliver",
        "domain must not depend-on driven"
    ));

    // Writing back what was read keeps the content (comments aside).
    let again: Areas = toml::from_str(&areas.to_toml().unwrap()).unwrap();
    assert_eq!(again, areas);
}
