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

#[test]
fn every_pinned_golden_facts_file_round_trips_through_the_store() {
    let golden_dir = repo_root().join("fixtures/arch-fixtures/golden");
    let Ok(rd) = std::fs::read_dir(&golden_dir) else {
        eprintln!(
            "no pinned arch-fixtures golden/ at {}: nothing to check yet (issue #9)",
            golden_dir.display()
        );
        return;
    };
    let mut n = 0;
    for entry in rd {
        let facts = entry.unwrap().path().join("facts.json");
        if facts.is_file() {
            round_trip(&facts);
            n += 1;
        }
    }
    eprintln!("{n} golden facts files round-tripped");
}
