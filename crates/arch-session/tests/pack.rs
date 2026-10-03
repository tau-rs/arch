//! The context pack on the sample service (#47, Done when: "a context-pack snapshot on the
//! smallsvc fixture facts"): the pinned golden facts (`golden/smallsvc/facts.json`) and
//! smallsvc's own `.arch/`, from the arch-fixtures checkout (`scripts/fetch-fixtures.sh`).
//!
//! The snapshots under `tests/snapshots/` are the pack as an agent reads it;
//! `ARCH_UPDATE_SNAPSHOT=1 cargo test -p arch-session --test pack` rewrites them.

use std::path::{Path, PathBuf};

use arch_facts::{ArchDir, ElementId, Facts, Gate, Group, Plan, SessionId};

fn fixtures() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/arch-fixtures");
    assert!(
        dir.join("golden/smallsvc/facts.json").is_file(),
        "{} has no golden/smallsvc/facts.json: run scripts/fetch-fixtures.sh",
        dir.display()
    );
    dir
}

fn facts() -> Facts {
    let path = fixtures().join("golden/smallsvc/facts.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn arch() -> ArchDir {
    ArchDir::of_repo(&fixtures().join("repos/smallsvc"))
}

/// Four elements in two groups: the shape of the replay scenarios. Groups are written by hand
/// here; shaping them is the engine's job.
fn plan() -> Plan {
    let mut plan = Plan::new(SessionId::new("refund-7f3a"), "refund a paid order");
    let e1 = plan
        .add_element(
            "add a Refunded status and its transition",
            "src/domain/order.rs",
        )
        .id
        .clone();
    let e2 = plan
        .add_element(
            "add refund to the payment gateway port",
            "src/ports/payment_gateway.rs",
        )
        .id
        .clone();
    let e3 = plan
        .add_element("the RefundOrder use case", "app")
        .id
        .clone();
    let e4 = plan
        .add_element(
            "POST /orders/:id/refund",
            "orderly::adapters::http::handlers::pay_order#fn",
        )
        .id
        .clone();
    let files = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    plan.elements[0].files = files(&["src/domain/order.rs"]);
    plan.elements[1].files = files(&["src/ports/payment_gateway.rs"]);
    plan.elements[2].files = files(&["src/app/refund.rs", "src/app/mod.rs"]);
    plan.elements[2].depends_on = vec![e1.clone(), e2.clone()];
    plan.elements[3].files = files(&["src/adapters/http/handlers.rs", "src/adapters/http/mod.rs"]);
    plan.elements[3].depends_on = vec![e3.clone()];
    let gate = Gate {
        commands: vec!["cargo test".into()],
        ..Gate::default()
    };
    plan.groups = vec![
        Group {
            name: "model".into(),
            elements: vec![e1, e2],
            gate: gate.clone(),
        },
        Group {
            name: "flow".into(),
            elements: vec![e3, e4],
            gate,
        },
    ];
    plan
}

fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    if std::env::var_os("ARCH_UPDATE_SNAPSHOT").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "{} is out of date; run ARCH_UPDATE_SNAPSHOT=1 cargo test -p arch-session --test pack\n--- actual ---\n{actual}",
        path.display()
    );
}

#[test]
fn the_element_pack_on_smallsvc() {
    let plan = plan();
    let e3 = plan.elements[2].id.clone();
    let pack = arch_session::pack::build(&facts(), &arch(), &plan, Some(&e3)).unwrap();
    snapshot("smallsvc-element.md", &pack);
}

#[test]
fn the_judge_pack_on_smallsvc() {
    let pack = arch_session::pack::build(&facts(), &arch(), &plan(), None).unwrap();
    snapshot("smallsvc-judge.md", &pack);
}

#[test]
fn area_descriptions_are_read_from_dot_arch() {
    let tmp = tempfile::tempdir().unwrap();
    let src = arch();
    let dst = ArchDir::at(tmp.path().join(".arch"));
    dst.write_areas(&src.read_areas().unwrap()).unwrap();
    dst.write_rules(&src.read_rules().unwrap()).unwrap();
    dst.write_allows(&src.read_allows().unwrap()).unwrap();
    dst.write_area_description(
        "app",
        "Use cases. One struct per flow, `run` is the flow.\n",
    )
    .unwrap();

    let pack = arch_session::pack::build(&facts(), &dst, &plan(), None).unwrap();

    assert!(
        pack.contains("### app\n\nUse cases. One struct per flow, `run` is the flow.\n"),
        "{pack}"
    );
}

#[test]
fn an_element_outside_the_plan_is_an_error() {
    let err = arch_session::pack::build(
        &facts(),
        &arch(),
        &plan(),
        Some(&ElementId::from_str_unchecked("deadbeef")),
    )
    .unwrap_err();
    assert!(matches!(err, arch_session::Error::Views(_)), "{err}");
}
