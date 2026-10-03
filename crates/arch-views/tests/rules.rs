//! Dependency rules on hand-built facts: one behaviour per test. No analyzer here.

use arch_facts::{
    Allow, Allows, Analyzer, AreaOverride, Areas, Column, Confidence, External, Facts, Item,
    ItemFlags, ItemKind, Level, Link, LinkFlags, LinkKind, PortKind, Repo, Rule, Rules, Span,
    Target, Unit, Visibility, Witness,
};
use arch_views::check_rules;

fn item(id: &str, file: &str, module: &str) -> Item {
    Item {
        id: id.into(),
        kind: ItemKind::Fn,
        name: id
            .rsplit("::")
            .next()
            .unwrap()
            .trim_end_matches("#fn")
            .into(),
        file: file.into(),
        span: Span {
            line: 1,
            col: 1,
            end_line: 2,
            end_col: 1,
        },
        crate_name: "svc".into(),
        module: module.into(),
        parent: None,
        visibility: Visibility::Pub,
        reexported: false,
        flags: ItemFlags::default(),
        scope: "unit:svc".into(),
    }
}

fn link(from: &str, to: Target, kind: LinkKind, confidence: Confidence, line: u32) -> Link {
    let file = if from.contains("::app::") {
        "src/app/pay.rs"
    } else {
        "src/http/handlers.rs"
    };
    Link {
        from: from.into(),
        to,
        member: None,
        kind,
        confidence,
        witness: Witness::Span {
            file: file.into(),
            line,
            col: None,
        },
        flags: LinkFlags::default(),
        reason: None,
    }
}

fn span_witness(file: &str) -> Witness {
    Witness::Declared {
        file: file.into(),
        line: 1,
    }
}

const PAY: &str = "svc::app::pay::impl PayOrder::run#fn";
const HANDLER: &str = "svc::http::handlers::pay#fn";
const PG_NEW: &str = "svc::postgres::impl PgRepo::new#fn";
const ORDER: &str = "svc::domain::order::Order#struct";

fn facts(links: Vec<Link>) -> Facts {
    let mut f = Facts::empty(
        Repo {
            name: "svc".into(),
            commit: "0".repeat(40),
            unit: Unit {
                id: "unit:svc".into(),
                main_target: "bin:svc".into(),
            },
        },
        Analyzer {
            name: "test".into(),
            version: "0".into(),
            degraded: vec![],
        },
    );
    f.items = vec![
        item(PAY, "src/app/pay.rs", "app::pay"),
        item(HANDLER, "src/http/handlers.rs", "http::handlers"),
        item(PG_NEW, "src/postgres/mod.rs", "postgres"),
        item(ORDER, "src/domain/order.rs", "domain::order"),
        item("svc::main#fn", "src/main.rs", ""),
    ];
    f.externals = vec![
        External {
            id: "external:sql:postgres".into(),
            name: "postgres".into(),
            kind: PortKind::Sql,
            touched: vec![],
            witness: span_witness("Cargo.toml"),
        },
        External {
            id: "external:crate:uuid".into(),
            name: "uuid".into(),
            kind: PortKind::Crate,
            touched: vec![],
            witness: span_witness("Cargo.toml"),
        },
    ];
    f.links = links;
    f
}

fn areas() -> Areas {
    let area = |name: &str, side, path: &str| AreaOverride {
        name: name.into(),
        paths: vec![path.into()],
        side: Some(side),
        order: None,
    };
    Areas {
        areas: vec![
            area("http", Column::Driving, "src/http/**"),
            area("app", Column::Domain, "src/app/**"),
            area("domain", Column::Domain, "src/domain/**"),
            area("postgres", Column::Driven, "src/postgres/**"),
        ],
        ..Areas::default()
    }
}

fn run(links: Vec<Link>, allows: Allows) -> arch_views::Report {
    check_rules(&facts(links), &areas(), &Rules::v1_template(), &allows).unwrap()
}

#[test]
fn a_resolved_link_from_domain_to_driven_blocks_with_its_witness() {
    let report = run(
        vec![link(
            PAY,
            Target::Item(PG_NEW.into()),
            LinkKind::Calls,
            Confidence::Resolved,
            12,
        )],
        Allows::default(),
    );
    assert_eq!(report.findings.len(), 1);
    let f = &report.findings[0];
    assert_eq!(f.rule, "domain must not depend-on driven");
    assert_eq!(f.level, Level::Block);
    assert!(f.blocks());
    assert_eq!(f.origin, "core");
    assert_eq!(f.site, "src/app/pay.rs::PayOrder::run");
    assert_eq!(f.target, "src/postgres/mod.rs::PgRepo::new");
    assert_eq!(
        (f.from_area.as_str(), f.target_area.as_str()),
        ("app", "postgres")
    );
    assert_eq!(
        f.witness,
        Witness::Span {
            file: "src/app/pay.rs".into(),
            line: 12,
            col: None
        }
    );
}

#[test]
fn a_guessed_or_declared_link_warns_and_never_blocks() {
    for confidence in [Confidence::Guessed, Confidence::Declared] {
        let report = run(
            vec![link(
                PAY,
                Target::Item(PG_NEW.into()),
                LinkKind::Calls,
                confidence,
                12,
            )],
            Allows::default(),
        );
        assert_eq!(report.findings[0].level, Level::Warn, "{confidence:?}");
        assert_eq!(report.blocking().count(), 0);
        assert_eq!(report.warnings().count(), 1);
    }
}

#[test]
fn links_with_the_grain_and_inside_one_side_are_clean() {
    let report = run(
        vec![
            link(
                HANDLER,
                Target::Item(PAY.into()),
                LinkKind::Calls,
                Confidence::Resolved,
                5,
            ),
            link(
                PG_NEW,
                Target::Item(ORDER.into()),
                LinkKind::UsesType,
                Confidence::Resolved,
                6,
            ),
            link(
                PAY,
                Target::Item(ORDER.into()),
                LinkKind::Constructs,
                Confidence::Resolved,
                7,
            ),
        ],
        Allows::default(),
    );
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
}

#[test]
fn io_externals_and_tables_count_as_externals_and_libraries_do_not() {
    let report = run(
        vec![
            link(
                PAY,
                Target::External("external:sql:postgres".into()),
                LinkKind::CallsOut,
                Confidence::Resolved,
                1,
            ),
            link(
                PAY,
                Target::Table("orders".into()),
                LinkKind::Reads,
                Confidence::Resolved,
                2,
            ),
            link(
                PAY,
                Target::External("external:crate:uuid".into()),
                LinkKind::UsesType,
                Confidence::Resolved,
                3,
            ),
            link(
                HANDLER,
                Target::Table("orders".into()),
                LinkKind::Reads,
                Confidence::Resolved,
                4,
            ),
        ],
        Allows::default(),
    );
    let got: Vec<(&str, &str)> = report
        .findings
        .iter()
        .map(|f| (f.rule.as_str(), f.target.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                "domain must not depend-on externals",
                "external:sql:postgres"
            ),
            ("domain must not depend-on externals", "table:orders"),
            ("driving must not depend-on externals", "table:orders"),
        ]
    );
}

#[test]
fn unplaced_items_test_links_and_test_code_have_no_findings() {
    let mut f = facts(vec![
        link(
            "svc::main#fn",
            Target::Item(PG_NEW.into()),
            LinkKind::Wires,
            Confidence::Resolved,
            1,
        ),
        link(
            PAY,
            Target::Item(PG_NEW.into()),
            LinkKind::Tests,
            Confidence::Resolved,
            2,
        ),
        link(
            "svc::app::pay::tests::fake#fn",
            Target::Item(PG_NEW.into()),
            LinkKind::Calls,
            Confidence::Resolved,
            3,
        ),
    ]);
    let mut test_fn = item(
        "svc::app::pay::tests::fake#fn",
        "src/app/pay.rs",
        "app::pay::tests",
    );
    test_fn.flags.cfg = Some("test".into());
    f.items.push(test_fn);
    let report = check_rules(&f, &areas(), &Rules::v1_template(), &Allows::default()).unwrap();
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
}

#[test]
fn an_allow_keeps_the_finding_marks_it_and_stops_it_blocking() {
    let allow = |target: Option<&str>| Allows {
        allows: vec![Allow {
            site: "src/app/pay.rs::PayOrder::run".into(),
            rule: "domain must not depend-on driven".into(),
            target: target.map(Into::into),
            reason: "until the port lands".into(),
            by: "titouan".into(),
            at: None,
        }],
    };
    let links = || {
        vec![link(
            PAY,
            Target::Item(PG_NEW.into()),
            LinkKind::Calls,
            Confidence::Resolved,
            12,
        )]
    };

    // target given as the type covers the method under it; no target covers the whole site
    for target in [
        Some("src/postgres/mod.rs::PgRepo"),
        Some("src/postgres/mod.rs::PgRepo::new"),
        None,
    ] {
        let report = run(links(), allow(target));
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].allowed.as_ref().unwrap().by, "titouan");
        assert_eq!(report.blocking().count(), 0, "{target:?}");
    }
    // another target, or a rule worded differently, does not cover it
    assert_eq!(
        run(links(), allow(Some("src/postgres/mod.rs::Pg")))
            .blocking()
            .count(),
        1
    );
    let mut other_rule = allow(None);
    other_rule.allows[0].rule = "domain must not depend-on driving".into();
    assert_eq!(run(links(), other_rule).blocking().count(), 1);
}

#[test]
fn a_rule_can_name_areas_instead_of_sides() {
    let rules = Rules {
        rules: vec![Rule {
            subject: "http".into(),
            must_not: "depend-on".into(),
            targets: vec!["postgres".into()],
            level: Level::Warn,
        }],
        ..Rules::default()
    };
    let f = facts(vec![link(
        HANDLER,
        Target::Item(PG_NEW.into()),
        LinkKind::Constructs,
        Confidence::Resolved,
        9,
    )]);
    let report = check_rules(&f, &areas(), &rules, &Allows::default()).unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].rule, "http must not depend-on postgres");
    assert_eq!(report.findings[0].level, Level::Warn);
}
