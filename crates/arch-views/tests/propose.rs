//! `propose_areas` on hand-built facts shaped like the two fixtures' tables in ADR 0027 and
//! ADR 0029. No analyzer here.

use arch_facts::{
    Analyzer, Column, ColumnRule, Confidence, Entry, EntryKind, External, Facts, Item, ItemFlags,
    ItemKind, Link, LinkFlags, LinkKind, PortKind, Repo, Span, Target, Unit, Visibility, Witness,
};
use arch_views::propose_areas;

struct Builder(Facts);

fn witness() -> Witness {
    Witness::Span {
        file: "src/lib.rs".into(),
        line: 1,
        col: None,
    }
}

impl Builder {
    fn new(main_target: &str) -> Self {
        let mut facts = Facts::empty(
            Repo {
                name: "svc".into(),
                commit: "0".repeat(40),
                unit: Unit {
                    id: "unit:svc".into(),
                    main_target: main_target.into(),
                },
            },
            Analyzer {
                name: "hand".into(),
                version: "0".into(),
                degraded: vec![],
            },
        );
        facts.externals = vec![
            External {
                id: "external:sql:postgres".into(),
                name: "postgres".into(),
                kind: PortKind::Sql,
                touched: vec![],
                witness: witness(),
            },
            External {
                id: "external:http:stripe".into(),
                name: "stripe".into(),
                kind: PortKind::Http,
                touched: vec![],
                witness: witness(),
            },
            External {
                id: "external:crate:uuid".into(),
                name: "uuid".into(),
                kind: PortKind::Crate,
                touched: vec![],
                witness: witness(),
            },
        ];
        let mut b = Builder(facts);
        b.item("svc::lib#mod", ItemKind::Mod, "src/lib.rs", "");
        b
    }

    fn item(&mut self, id: &str, kind: ItemKind, file: &str, module: &str) -> &mut Self {
        self.0.items.push(Item {
            id: id.into(),
            kind,
            name: id.into(),
            file: file.into(),
            span: Span {
                line: 1,
                col: 1,
                end_line: 1,
                end_col: 1,
            },
            crate_name: "svc".into(),
            module: module.into(),
            parent: None,
            visibility: Visibility::Pub,
            reexported: false,
            flags: ItemFlags::default(),
            scope: "unit:svc".into(),
        });
        self
    }

    /// A function `f` in `module`, in `file`; returns its id.
    fn func(&mut self, module: &str, file: &str) -> String {
        let id = format!("svc::{module}::f#fn");
        if !self.0.items.iter().any(|i| i.id == id) {
            self.item(&id, ItemKind::Fn, file, module);
        }
        id
    }

    fn link(&mut self, from: &str, to: Target, kind: LinkKind) -> &mut Self {
        self.0.links.push(Link {
            from: from.into(),
            to,
            member: None,
            kind,
            confidence: Confidence::Guessed,
            witness: witness(),
            flags: LinkFlags::default(),
            reason: None,
        });
        self
    }

    fn plain(&mut self, module: &str, file: &str) -> &mut Self {
        self.func(module, file);
        self
    }

    fn entry(&mut self, module: &str, file: &str, kind: EntryKind) -> &mut Self {
        let item = self.func(module, file);
        self.0.entries.push(Entry {
            item,
            kind,
            framework: None,
            confidence: Confidence::Guessed,
            witness: witness(),
        });
        self
    }

    fn touches(&mut self, module: &str, file: &str, to: Target) -> &mut Self {
        let from = self.func(module, file);
        let kind = if matches!(to, Target::Table(_)) {
            LinkKind::Reads
        } else {
            LinkKind::CallsOut
        };
        self.link(&from, to, kind)
    }

    fn implements(&mut self, module: &str, file: &str, tr: &str) -> &mut Self {
        let from = self.func(module, file);
        self.link(&from, Target::Item(tr.into()), LinkKind::Implements)
    }
}

fn table(areas: &arch_facts::Areas) -> Vec<(Option<Column>, &str, Vec<&str>, u32)> {
    areas
        .areas
        .iter()
        .map(|a| {
            (
                a.side,
                a.name.as_str(),
                a.paths.iter().map(String::as_str).collect(),
                a.order.unwrap(),
            )
        })
        .collect()
}

const REPO: &str = "svc::ports::OrderRepository#trait";

/// ADR 0029, "Result on smallsvc", computed column.
#[test]
fn a_hexagon_like_smallsvc() {
    let mut b = Builder::new("bin:orderly");
    b.item(
        REPO,
        ItemKind::Trait,
        "src/ports/order_repo.rs",
        "ports::order_repo",
    );
    b.entry(
        "adapters::http::handlers",
        "src/adapters/http/handlers.rs",
        EntryKind::Framework,
    )
    .touches(
        "adapters::postgres::orders",
        "src/adapters/postgres/orders.rs",
        Target::Table("orders".into()),
    )
    .implements(
        "adapters::postgres::orders",
        "src/adapters/postgres/orders.rs",
        REPO,
    )
    .implements("adapters::memory", "src/adapters/memory/mod.rs", REPO)
    .touches(
        "adapters::stripe",
        "src/adapters/stripe/mod.rs",
        Target::External("external:http:stripe".into()),
    )
    .plain("adapters", "src/adapters/mod.rs")
    .plain("app::pay", "src/app/pay.rs")
    .touches(
        "app::pay",
        "src/app/pay.rs",
        Target::External("external:crate:uuid".into()),
    )
    .plain("config", "src/config.rs")
    .plain("domain::order", "src/domain/order.rs")
    .entry("worker", "src/worker.rs", EntryKind::SpawnedWorker);

    let areas = propose_areas(&b.0);

    assert_eq!(areas.rule, Some(ColumnRule::Hexagon));
    assert_eq!(areas.main_bin.as_deref(), Some("orderly"));
    use Column::*;
    assert_eq!(
        table(&areas),
        vec![
            (Some(Driving), "http", vec!["src/adapters/http/**"], 1),
            (Some(Driving), "worker", vec!["src/worker.rs"], 2),
            (Some(Domain), "app", vec!["src/app/**"], 1),
            (Some(Domain), "config", vec!["src/config.rs"], 2),
            (Some(Domain), "domain", vec!["src/domain/**"], 3),
            (Some(Domain), "ports", vec!["src/ports/**"], 4),
            (Some(Driven), "memory", vec!["src/adapters/memory/**"], 1),
            (
                Some(Driven),
                "postgres",
                vec!["src/adapters/postgres/**"],
                2
            ),
            (Some(Driven), "stripe", vec!["src/adapters/stripe/**"], 3),
        ]
    );
}

/// ADR 0029, "Result on zero2prod": no split when every child of `routes` is driving; a routes
/// link's handler end counts as an entry; single files are areas.
#[test]
fn a_flat_service_like_zero2prod() {
    let mut b = Builder::new("bin:zero2prod");
    let handler = b.func("routes::login", "src/routes/login.rs");
    b.link("svc::lib#mod", Target::Item(handler), LinkKind::Routes)
        .entry(
            "routes::admin",
            "src/routes/admin/mod.rs",
            EntryKind::Framework,
        )
        .touches(
            "routes::admin",
            "src/routes/admin/mod.rs",
            Target::Table("users".into()),
        )
        .entry(
            "issue_delivery_worker",
            "src/issue_delivery_worker.rs",
            EntryKind::SpawnedWorker,
        )
        .touches(
            "authentication::password",
            "src/authentication/password.rs",
            Target::Table("users".into()),
        )
        .plain(
            "authentication::middleware",
            "src/authentication/middleware.rs",
        )
        .touches(
            "email_client",
            "src/email_client.rs",
            Target::External("external:http:stripe".into()),
        )
        .plain("telemetry", "src/telemetry.rs")
        .plain("domain::subscriber", "src/domain/subscriber.rs");

    use Column::*;
    assert_eq!(
        table(&propose_areas(&b.0)),
        vec![
            (
                Some(Driving),
                "issue_delivery_worker",
                vec!["src/issue_delivery_worker.rs"],
                1
            ),
            (Some(Driving), "routes", vec!["src/routes/**"], 2),
            (Some(Domain), "domain", vec!["src/domain/**"], 1),
            (Some(Domain), "telemetry", vec!["src/telemetry.rs"], 2),
            (
                Some(Driven),
                "authentication",
                vec!["src/authentication/**"],
                1
            ),
            (Some(Driven), "email_client", vec!["src/email_client.rs"], 2),
        ]
    );
}

/// arch-design#29: the module that mounts the handlers holds an entry too, so zero2prod's
/// `startup` is driving. The crate root has no area: a route registered there places only its
/// handler.
#[test]
fn the_module_that_registers_a_route_is_driving() {
    let mut b = Builder::new("bin:svc");
    let home = b.func("routes", "src/routes.rs");
    let health = b.func("health", "src/health.rs");
    let startup = b.func("startup", "src/startup.rs");
    b.link(&startup, Target::Item(home), LinkKind::Routes)
        .link("svc::lib#mod", Target::Item(health), LinkKind::Routes)
        .plain("config", "src/config.rs");
    // `main` in the crate root makes the unit a hexagon and places nothing
    b.0.entries.push(Entry {
        item: "svc::lib#mod".into(),
        kind: EntryKind::Main,
        framework: None,
        confidence: Confidence::Guessed,
        witness: witness(),
    });

    use Column::*;
    assert_eq!(
        table(&propose_areas(&b.0)),
        vec![
            (Some(Driving), "health", vec!["src/health.rs"], 1),
            (Some(Driving), "routes", vec!["src/routes.rs"], 2),
            (Some(Driving), "startup", vec!["src/startup.rs"], 3),
            (Some(Domain), "config", vec!["src/config.rs"], 1),
        ]
    );
}

#[test]
fn a_unit_without_an_entry_is_layers_with_order_only() {
    let mut b = Builder::new("lib");
    b.touches("walk", "src/walk.rs", Target::Table("t".into()))
        .plain("glob", "src/glob/mod.rs")
        .plain("Zeta", "src/Zeta.rs");
    let areas = propose_areas(&b.0);
    assert_eq!(areas.rule, Some(ColumnRule::Layers));
    assert_eq!(areas.main_bin, None);
    // byte order: uppercase before lowercase
    assert_eq!(
        table(&areas),
        vec![
            (None, "Zeta", vec!["src/Zeta.rs"], 1),
            (None, "glob", vec!["src/glob/**"], 2),
            (None, "walk", vec!["src/walk.rs"], 3),
        ]
    );
}

#[test]
fn a_split_child_is_prefixed_when_its_name_is_taken() {
    let mut b = Builder::new("bin:svc");
    for feature in ["orders", "users"] {
        b.entry(
            &format!("{feature}::handlers"),
            &format!("src/{feature}/handlers.rs"),
            EntryKind::Framework,
        )
        .touches(
            &format!("{feature}::repo"),
            &format!("src/{feature}/repo.rs"),
            Target::Table("t".into()),
        );
    }
    let areas = propose_areas(&b.0);
    let names: Vec<&str> = areas.areas.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["handlers", "users::handlers", "repo", "users::repo"]
    );
    assert_eq!(areas.areas[1].paths, vec!["src/users/handlers.rs"]);
}

#[test]
fn test_code_and_other_roots_place_nothing() {
    let mut b = Builder::new("bin:svc");
    b.entry("api", "src/api.rs", EntryKind::Framework)
        .plain("core", "src/core.rs");
    // a test-only module that opens a database must not make `core` driven
    let fake = "svc::core::tests::fake#fn";
    b.item(fake, ItemKind::Fn, "src/core.rs", "core::tests");
    b.0.items.last_mut().unwrap().flags.cfg = Some("test".into());
    b.link(fake, Target::Table("t".into()), LinkKind::Reads);
    // an integration test file is outside the root crate's modules
    b.item(
        "svc[test:flows]::flows::t#fn",
        ItemKind::Fn,
        "tests/flows.rs",
        "flows",
    );

    use Column::*;
    assert_eq!(
        table(&propose_areas(&b.0)),
        vec![
            (Some(Driving), "api", vec!["src/api.rs"], 1),
            (Some(Domain), "core", vec!["src/core.rs"], 1)
        ]
    );
}
