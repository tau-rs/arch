//! The syntax-level pass on `repos/smallsvc` from the arch-fixtures pin: what the fixture's README
//! says arch should find "by construction" is found, the document is valid against the published
//! schema, and the store reassembles it unchanged.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arch_analyze::{Commits, Options, SYNTAX_REASON, analyze, index};
use arch_facts::*;

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
        commit: Some("f".repeat(40)),
        commits: Commits::None,
        ..Default::default()
    }
}

fn facts() -> Facts {
    analyze(&smallsvc(), &options()).unwrap()
}

fn has_link(f: &Facts, from: &str, kind: LinkKind, to: &Target) -> bool {
    f.links
        .iter()
        .any(|l| l.from.ends_with(from) && l.kind == kind && &l.to == to)
}

fn item(id: &str) -> Target {
    Target::Item(id.into())
}

#[test]
fn the_unit_is_the_orderly_bin_and_its_lib() {
    let f = facts();
    assert_eq!(
        f.repo.unit,
        Unit {
            id: "unit:smallsvc".into(),
            main_target: "bin:orderly".into()
        }
    );
    assert_eq!(f.crates.len(), 1);
    assert_eq!(f.crates[0].status, CrateStatus::Analyzed);
    assert_eq!(f.crates[0].targets, ["bin:orderly", "lib", "test:flows"]);
    assert_eq!(
        f.analyzer.degraded,
        [Degraded {
            crate_name: "orderly".into(),
            reason: SYNTAX_REASON.into()
        }]
    );
    // The integration test target is outside the bin's closure: none of its items appear.
    assert!(f.items.iter().all(|i| i.file.starts_with("src/")));
    assert!(
        f.items
            .iter()
            .any(|i| i.id == "orderly[bin:orderly]::main#fn" && i.flags.entry)
    );
}

#[test]
fn items_are_everything_rust_analyzer_calls_an_item() {
    let f = facts();
    let count = |k: ItemKind| f.items.iter().filter(|i| i.kind == k).count();
    // golden/smallsvc/sizes.json: fn 129 (methods included), struct 53, enum 17, trait 5.
    assert_eq!(
        (
            count(ItemKind::Fn),
            count(ItemKind::Struct),
            count(ItemKind::Enum),
            count(ItemKind::Trait)
        ),
        (129, 53, 17, 5)
    );
    let method = f
        .items
        .iter()
        .find(|i| {
            i.id == "orderly::adapters::postgres::outbox::impl Outbox for PgOutbox::enqueue#fn"
        })
        .unwrap();
    assert_eq!(
        method.parent.as_deref(),
        Some("orderly::adapters::postgres::outbox::impl Outbox for PgOutbox#impl")
    );
    let ids: BTreeSet<&str> = f.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids.len(), f.items.len(), "item ids are unique");
    assert!(
        f.items
            .iter()
            .any(|i| i.id == "orderly::app::Services#struct" && i.reexported),
        "lib.rs re-exports Services"
    );
}

#[test]
fn a_port_has_two_adapters() {
    let f = facts();
    let port = item("orderly::ports::order_repo::OrderRepository#trait");
    for adapter in [
        "postgres::orders::impl OrderRepository for PgOrderRepository#impl",
        "memory::impl OrderRepository for InMemoryOrderRepository#impl",
    ] {
        assert!(
            has_link(&f, adapter, LinkKind::Implements, &port),
            "{adapter}"
        );
    }
    // The use case calls the port, not an adapter.
    assert!(has_link(
        &f,
        "app::pay::impl PayOrder::run#fn",
        LinkKind::CallsPort,
        &item("orderly::ports::order_repo::OrderRepository::get#fn")
    ));
    // main wires the postgres adapter to it.
    assert!(has_link(
        &f,
        "main#fn",
        LinkKind::Wires,
        &item(
            "orderly::adapters::postgres::orders::impl OrderRepository for PgOrderRepository#impl"
        )
    ));
}

#[test]
fn routes_lead_to_handlers_behind_the_middleware_stack() {
    let f = facts();
    let route = f
        .links
        .iter()
        .find(|l| {
            l.kind == LinkKind::Routes
                && l.to == item("orderly::adapters::http::handlers::pay_order#fn")
        })
        .unwrap();
    assert_eq!(route.from, "orderly::adapters::http::router#fn");
    assert_eq!(
        route.reason.as_deref(),
        Some(
            "route `POST /orders/:id/pay`; middleware, outermost first: TraceLayer → request_id → require_api_key"
        )
    );
    let health = f
        .links
        .iter()
        .find(|l| {
            l.kind == LinkKind::Routes
                && l.to == item("orderly::adapters::http::handlers::health#fn")
        })
        .unwrap();
    assert!(
        health
            .reason
            .as_deref()
            .unwrap()
            .ends_with("TraceLayer → request_id"),
        "the api key layer does not cover /health"
    );
    let driving: Vec<&str> = f
        .ports
        .iter()
        .filter(|p| p.side == Side::Driving)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(
        driving,
        [
            "GET /health",
            "GET /orders/:id",
            "POST /orders",
            "POST /orders/:id/pay",
            "POST /orders/:id/ship"
        ]
    );
    let handlers = f
        .entries
        .iter()
        .filter(|e| e.framework == Some(Framework::Axum))
        .count();
    assert_eq!(handlers, 5);
}

#[test]
fn the_spawned_outbox_worker_is_an_entry() {
    let f = facts();
    let worker = "orderly::worker::run_outbox_worker#fn";
    assert!(has_link(&f, "main#fn", LinkKind::HandsOff, &item(worker)));
    assert!(
        !has_link(&f, "main#fn", LinkKind::Calls, &item(worker)),
        "a spawn is not a direct call"
    );
    let entry = f
        .entries
        .iter()
        .find(|e| e.item == worker)
        .expect("ADR 0028: spawned worker entry");
    assert!(matches!(&entry.witness, Witness::Span { file, .. } if file == "src/main.rs"));
    assert!(f.items.iter().any(|i| i.id == worker && i.flags.entry));
    // At syntax depth nothing is resolved, the entry included.
    assert_eq!(
        (entry.kind, entry.confidence),
        (EntryKind::SpawnedWorker, Confidence::Guessed)
    );
    assert!(
        f.entries.iter().all(|e| !e.item.contains("::tests::")),
        "a test is not an entry"
    );
    // The loop's body is inside `tokio::select!`; the call it makes is still seen.
    assert!(has_link(
        &f,
        worker,
        LinkKind::Calls,
        &item("orderly::app::notify::impl NotifyCustomer::drain#fn")
    ));
}

#[test]
fn migrations_name_the_tables_and_outbox_is_a_queue() {
    let f = facts();
    let names: Vec<&str> = f.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["order_lines", "orders", "outbox", "payments", "shipments"]
    );
    let outbox = &f.tables[2];
    let q = outbox.queue.as_ref().expect("outbox is used as a queue");
    assert_eq!(
        q.inserters,
        ["orderly::adapters::postgres::outbox::impl Outbox for PgOutbox::enqueue#fn"]
    );
    assert_eq!(
        q.dequeuers,
        ["orderly::adapters::postgres::outbox::impl Outbox for PgOutbox::dequeue#fn"]
    );
    assert!(
        f.tables
            .iter()
            .filter(|t| t.name != "outbox")
            .all(|t| t.queue.is_none())
    );
    let dequeue = f
        .links
        .iter()
        .find(|l| l.kind == LinkKind::Queues && l.from.ends_with("dequeue#fn"))
        .unwrap();
    assert_eq!(
        (dequeue.to.clone(), dequeue.flags.access),
        (Target::Table("outbox".into()), Some(Access::Read))
    );
}

#[test]
fn externals_are_the_database_three_http_apis_the_log_and_the_crates_touched() {
    let f = facts();
    let ids: BTreeSet<&str> = f.externals.iter().map(|e| e.id.as_str()).collect();
    for id in [
        "external:sql:postgres",
        "external:http:stripe",
        "external:http:carrier",
        "external:http:email",
        "external:tty:log",
        "external:crate:sqlx",
        "external:crate:axum",
    ] {
        assert!(ids.contains(id), "{id} in {ids:?}");
    }
    let pg = f
        .externals
        .iter()
        .find(|e| e.id == "external:sql:postgres")
        .unwrap();
    assert_eq!(
        pg.touched,
        ["order_lines", "orders", "outbox", "payments", "shipments"]
    );
    assert!(matches!(&pg.witness, Witness::Declared { file, .. } if file == "Cargo.toml"));
    assert!(has_link(
        &f,
        "impl PaymentGateway for StripeGateway::charge#fn",
        LinkKind::CallsOut,
        &Target::External("external:http:stripe".into())
    ));
    assert!(f.ports.iter().any(|p| p.id == "port:sql:postgres"
        && p.section == Section::DataStores
        && p.side == Side::Driven));
}

#[test]
fn the_allowed_violation_is_visible_as_a_link() {
    // .arch/allows: app::notify names the driven adapter LogNotifier.
    let f = facts();
    let l = f
        .links
        .iter()
        .find(|l| {
            l.from.ends_with("NotifyCustomer::deliver#fn")
                && l.to == item("orderly::adapters::email::LogNotifier#struct")
        })
        .expect("deliver → LogNotifier");
    assert!(matches!(&l.witness, Witness::Span { file, .. } if file == "src/app/notify.rs"));
}

#[test]
fn every_link_is_guessed_says_why_and_points_at_something_that_exists() {
    let f = facts();
    let items: BTreeSet<&str> = f.items.iter().map(|i| i.id.as_str()).collect();
    let externals: BTreeSet<&str> = f.externals.iter().map(|e| e.id.as_str()).collect();
    let tables: BTreeSet<&str> = f.tables.iter().map(|t| t.name.as_str()).collect();
    for l in &f.links {
        assert_eq!(l.confidence, Confidence::Guessed, "{l:?}");
        assert!(l.reason.as_deref().is_some_and(|r| !r.is_empty()), "{l:?}");
        assert!(items.contains(l.from.as_str()), "from {l:?}");
        match &l.to {
            Target::Item(id) => assert!(items.contains(id.as_str()), "to {l:?}"),
            Target::External(id) => assert!(externals.contains(id.as_str()), "to {l:?}"),
            Target::Table(t) => assert!(tables.contains(t.as_str()), "to {l:?}"),
            Target::Port(_) => {}
        }
    }
    for e in &f.entries {
        assert!(items.contains(e.item.as_str()), "{e:?}");
    }
    // Members name variants and fields, which are not items.
    assert!(
        f.links
            .iter()
            .any(|l| l.kind == LinkKind::MatchesOn && l.member.as_deref() == Some("OrderPaid"))
    );
    assert!(
        f.links
            .iter()
            .any(|l| l.kind == LinkKind::Holds && l.member.is_some())
    );
}

#[test]
fn the_document_is_valid_stable_and_survives_the_store() {
    let f = facts();
    let json = serde_json::to_value(&f).unwrap();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/facts.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&json)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .take(5)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");

    // Same input, same bytes.
    let again = serde_json::to_string(&facts()).unwrap();
    assert_eq!(serde_json::to_string(&f).unwrap(), again);

    // The store flow: every file of the tree has its delta, and a second tree at the same
    // hashes needs nothing recomputed.
    let mut store = Store::in_memory().unwrap();
    let key = index(&smallsvc(), &options(), &mut store).unwrap();
    assert!(store.missing_file_facts(&key).unwrap().is_empty());
    assert_eq!(store.facts(&key).unwrap().unwrap(), f);
    let main = store
        .tree_files(&key)
        .unwrap()
        .into_iter()
        .find(|(p, _)| p == Path::new("src/main.rs"))
        .unwrap();
    let delta = store.file_facts(&main.1).unwrap().unwrap();
    assert_eq!(delta.degraded.as_deref(), Some(SYNTAX_REASON));
    assert!(delta.items.iter().any(|i| i.name == "main"));
}
