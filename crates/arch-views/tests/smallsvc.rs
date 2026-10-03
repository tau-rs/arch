//! The sample service's own `.arch/` files, read from the pinned arch-fixtures checkout
//! (`scripts/fetch-fixtures.sh`), against the links its one planted violation produces:
//! `app::notify` reaching for `adapters::email::LogNotifier`, listed in `.arch/allows`.
//! The links are hand-built here, copied from the analyzer's output; no analyzer in this crate.

use std::path::PathBuf;

use arch_facts::{
    Analyzer, ArchDir, Confidence, Facts, Item, ItemFlags, ItemKind, Link, LinkFlags, LinkKind,
    Repo, Span, Target, Unit, Visibility, Witness,
};
use arch_views::check_rules;

fn smallsvc() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        dir.join(".arch/rules").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        dir.display()
    );
    dir
}

fn item(id: &str, kind: ItemKind, file: &str, module: &str, parent: Option<&str>) -> Item {
    Item {
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
        crate_name: "orderly".into(),
        module: module.into(),
        parent: parent.map(Into::into),
        visibility: Visibility::Pub,
        reexported: false,
        flags: ItemFlags::default(),
        scope: "unit:orderly".into(),
    }
}

#[test]
fn the_planted_violation_is_found_and_covered_by_the_allow() {
    let arch = ArchDir::of_repo(&smallsvc());
    let (areas, rules, allows) = (
        arch.read_areas().unwrap(),
        arch.read_rules().unwrap(),
        arch.read_allows().unwrap(),
    );

    const DELIVER: &str = "orderly::app::notify::impl NotifyCustomer::deliver#fn";
    const LOG: &str = "orderly::adapters::email::LogNotifier#struct";
    const SEND: &str = "orderly::adapters::email::impl Notifier for LogNotifier::send#fn";
    let mut facts = Facts::empty(
        Repo {
            name: "orderly".into(),
            commit: "0".repeat(40),
            unit: Unit {
                id: "unit:orderly".into(),
                main_target: "bin:orderly".into(),
            },
        },
        Analyzer {
            name: "hand".into(),
            version: "0".into(),
            degraded: vec![],
        },
    );
    facts.items = vec![
        item(
            DELIVER,
            ItemKind::Fn,
            "src/app/notify.rs",
            "app::notify",
            Some("orderly::app::notify::impl NotifyCustomer#impl"),
        ),
        item(
            LOG,
            ItemKind::Struct,
            "src/adapters/email/mod.rs",
            "adapters::email",
            None,
        ),
        item(
            SEND,
            ItemKind::Fn,
            "src/adapters/email/mod.rs",
            "adapters::email",
            None,
        ),
    ];
    let link = |to: &str, kind, col| Link {
        from: DELIVER.into(),
        to: Target::Item(to.into()),
        member: None,
        kind,
        confidence: Confidence::Resolved,
        witness: Witness::Span {
            file: "src/app/notify.rs".into(),
            line: 68,
            col: Some(col),
        },
        flags: LinkFlags::default(),
        reason: None,
    };
    facts.links = vec![
        link(LOG, LinkKind::Constructs, 13),
        link(SEND, LinkKind::Calls, 25),
    ];

    let report = check_rules(&facts, &areas, &rules, &allows).unwrap();

    assert_eq!(report.findings.len(), 2, "{:#?}", report.findings);
    for f in &report.findings {
        assert_eq!(f.rule, "domain must not depend-on driven");
        assert_eq!(f.site, "src/app/notify.rs::NotifyCustomer::deliver");
        assert_eq!(
            (f.from_area.as_str(), f.target_area.as_str()),
            ("app", "email")
        );
        assert!(f.allowed.is_some(), "not covered by .arch/allows: {f:#?}");
    }
    assert_eq!(report.blocking().count(), 0);
}
