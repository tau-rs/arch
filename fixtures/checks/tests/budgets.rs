//! `budgets.toml` is read by `arch` and `arch-app` CI: it must parse, name the reference machine, and carry
//! the six budgets of ADR 0026, one benchmark each.

use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Budgets {
    reference_machine: ReferenceMachine,
    budget: Vec<Budget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceMachine {
    name: String,
    runs_on: String,
    cpus: u32,
    memory_gb: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Budget {
    name: String,
    benchmark: String,
    below: Option<f64>,
    at_least: Option<f64>,
    unit: String,
    measures: String,
    fixture: String,
    runs_in: String,
}

fn budgets() -> Budgets {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/budgets.toml");
    toml::from_str(&std::fs::read_to_string(path).expect("budgets.toml exists")).expect("budgets.toml parses")
}

#[test]
fn reference_machine_is_named_and_pinned() {
    let m = budgets().reference_machine;
    assert!(!m.name.is_empty());
    assert!(!m.runs_on.is_empty() && !m.runs_on.ends_with("-latest"), "pin the runner label, got {:?}", m.runs_on);
    assert!(m.cpus > 0 && m.memory_gb > 0);
}

#[test]
fn six_budgets_one_benchmark_each() {
    let b = budgets().budget;
    let names: BTreeSet<&str> = b.iter().map(|b| b.benchmark.as_str()).collect();
    assert_eq!(names.len(), b.len(), "a benchmark appears twice");
    let expected = ["first_index_cold", "recompute_one_file", "memory_rss_peak", "fold_expand", "first_paint", "pan_zoom_fps"];
    assert_eq!(names, expected.into_iter().collect(), "the six benchmarks of ADR 0026");
}

#[test]
fn every_budget_has_one_threshold_an_owner_and_a_measure() {
    for b in budgets().budget {
        assert!(b.below.is_some() != b.at_least.is_some(), "{}: exactly one of below / at_least", b.benchmark);
        assert!(b.below.or(b.at_least).is_some_and(|n| n > 0.0), "{}: threshold must be positive", b.benchmark);
        assert!(["tau-rs/arch", "tau-rs/arch-app"].contains(&b.runs_in.as_str()), "{}: runs_in {:?}", b.benchmark, b.runs_in);
        for (field, value) in [("name", &b.name), ("unit", &b.unit), ("measures", &b.measures), ("fixture", &b.fixture)] {
            assert!(!value.is_empty(), "{}: empty {field}", b.benchmark);
        }
    }
}
