//! The context pack (ADR 0005): the one text the planner, the sub-agents, the judge and Ask read,
//! so an agent and a person see the same picture of the unit.
//!
//! A pure function of the facts, the three `.arch/` files, the area descriptions and the plan.
//! It holds, in this order:
//! - the unit's map as text: areas by side, ports, entries, and the links between areas (counted
//!   by kind, under the same reading the rules use: no tests, no test code, libraries do not count);
//! - the rules;
//! - the open findings (not covered by `.arch/allows`);
//! - the area descriptions (`.arch/areas/<name>.md`), verbatim;
//! - for a sub-agent, its element: position, the files it may write, and which areas it may and
//!   may not depend on; for the judge, the whole plan (ADR 0013: it verdicts against the same pack).
//!
//! Every list is in a fixed order, so the same inputs give the same bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use arch_facts::{
    Allows, Areas, Column, ColumnRule, Element, ElementId, EntryKind, External, Facts, Framework,
    Item, LinkKind, Plan, Port, Rules, Side, Witness,
};

use crate::Error;
use crate::findings::{EXTERNALS, Finding, check_rules, far_end};
use crate::placement::{Placement, Placements, side_name};
use crate::propose::is_test_code;

/// What the pack is built from.
#[derive(Debug, Clone, Copy)]
pub struct PackInput<'a> {
    /// The unit's facts.
    pub facts: &'a Facts,
    /// `.arch/areas.toml`.
    pub areas: &'a Areas,
    /// `.arch/rules`.
    pub rules: &'a Rules,
    /// `.arch/allows`.
    pub allows: &'a Allows,
    /// `.arch/areas/<name>.md`, by area name; areas without one are absent.
    pub descriptions: &'a BTreeMap<String, String>,
    /// The session's plan.
    pub plan: &'a Plan,
    /// The element whose sub-agent reads the pack; `None` for the judge, who reads the whole plan.
    pub element: Option<&'a ElementId>,
}

/// Render the context pack as Markdown.
pub fn context_pack(input: &PackInput<'_>) -> Result<String, Error> {
    let element = match input.element {
        Some(id) => Some(
            input
                .plan
                .element(id)
                .ok_or_else(|| Error::UnknownElement(id.to_string()))?,
        ),
        None => None,
    };
    let placements = Placements::new(input.areas)?;
    let report = check_rules(input.facts, input.areas, input.rules, input.allows)?;

    let mut out = String::new();
    header(&mut out, input.facts);
    map(&mut out, input, &placements);
    rules(&mut out, input.rules);
    findings(&mut out, &report.findings);
    descriptions(&mut out, input);
    match element {
        Some(element) => element_section(&mut out, input, &placements, element),
        None => plan_section(&mut out, input, &placements),
    }
    Ok(out)
}

fn header(out: &mut String, facts: &Facts) {
    let repo = &facts.repo;
    let _ = writeln!(out, "# Context pack · {}\n", repo.name);
    let _ = writeln!(
        out,
        "Unit `{}`, built from `{}`, at `{}`.",
        repo.unit.id, repo.unit.main_target, repo.commit
    );
    if facts.analyzer.degraded.is_empty() {
        out.push_str("Facts are type-checked.\n");
    } else {
        for d in &facts.analyzer.degraded {
            let _ = writeln!(
                out,
                "Crate `{}` is not type-checked ({}): its facts are guessed.",
                d.crate_name, d.reason
            );
        }
    }
    out.push('\n');
}

fn map(out: &mut String, input: &PackInput<'_>, placements: &Placements) {
    let facts = input.facts;
    out.push_str("## Map\n\n### Areas\n\n");
    let rule = input.areas.rule.map_or("not set", |r| match r {
        ColumnRule::Hexagon => "hexagon (driving → domain → driven → externals)",
        ColumnRule::Layers => "layers (public API → internals → leaves)",
    });
    let _ = writeln!(out, "Column rule: {rule}.\n");

    let mut items_in: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unplaced: BTreeSet<&str> = BTreeSet::new();
    for item in facts.items.iter().filter(|i| !is_test_code(i)) {
        match placements.of(item) {
            Some(p) => *items_in.entry(p.area.as_str()).or_default() += 1,
            None => {
                unplaced.insert(item.file.as_str());
            }
        }
    }
    let mut areas: Vec<_> = input.areas.areas.iter().collect();
    areas.sort_by_key(|a| {
        (
            a.side.map_or(u8::MAX, column_rank),
            a.order.unwrap_or(u32::MAX),
            a.name.as_str(),
        )
    });
    for area in areas {
        let side = area.side.map_or("no side", side_name);
        let _ = writeln!(
            out,
            "- **{}** · {} · {} · {} items",
            area.name,
            side,
            area.paths.join(", "),
            items_in.get(area.name.as_str()).copied().unwrap_or(0)
        );
    }
    if !unplaced.is_empty() {
        let files: Vec<_> = unplaced.iter().map(|f| format!("`{f}`")).collect();
        let _ = writeln!(
            out,
            "- outside every area (no rule applies): {}",
            files.join(", ")
        );
    }

    out.push_str("\n### Ports\n\n");
    if facts.ports.is_empty() {
        out.push_str("None.\n");
    }
    for side in [Side::Driving, Side::Driven] {
        for port in facts.ports.iter().filter(|p| p.side == side) {
            port_line(out, port);
        }
    }

    out.push_str("\n### Entries\n\n");
    if facts.entries.is_empty() {
        out.push_str("None.\n");
    }
    let items: BTreeMap<&str, &Item> = facts.items.iter().map(|i| (i.id.as_str(), i)).collect();
    for entry in &facts.entries {
        let how = match (&entry.kind, &entry.framework) {
            (EntryKind::Main, _) => "main".to_string(),
            (EntryKind::SpawnedWorker, _) => "spawned worker".to_string(),
            (EntryKind::Framework, Some(f)) => format!("framework ({})", framework_name(f)),
            (EntryKind::Framework, None) => "framework".to_string(),
        };
        let area = items
            .get(entry.item.as_str())
            .and_then(|i| placements.of(i))
            .map_or("outside every area".to_string(), |p| {
                format!("in `{}`", p.area)
            });
        let _ = writeln!(out, "- {how} · `{}` · {area}", entry.item);
    }

    out.push_str("\n### Links between areas\n\n");
    let between = links_between(facts, placements);
    if between.is_empty() {
        out.push_str("None.\n");
    }
    for ((from, to), kinds) in &between {
        let counts: Vec<_> = kinds
            .iter()
            .map(|(rank, n)| format!("{} {n}", kind_name(LinkKind::ALL[*rank])))
            .collect();
        let _ = writeln!(out, "- {from} → {to}: {}", counts.join(", "));
    }
    out.push('\n');
}

fn port_line(out: &mut String, port: &Port) {
    let side = match port.side {
        Side::Driving => "driving",
        Side::Driven => "driven",
    };
    let behind = port
        .external
        .as_deref()
        .map_or(String::new(), |e| format!(" · behind `{e}`"));
    let _ = writeln!(
        out,
        "- {side} · `{}` · {}{behind}",
        port.id,
        kebab(&format!("{:?}", port.section))
    );
}

/// Links between two different areas (or from an area to `externals`), counted by kind in
/// schema order.
fn links_between<'a>(
    facts: &'a Facts,
    placements: &'a Placements,
) -> BTreeMap<(String, String), BTreeMap<usize, usize>> {
    let items: BTreeMap<&str, &Item> = facts.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let externals: BTreeMap<&str, &External> =
        facts.externals.iter().map(|e| (e.id.as_str(), e)).collect();
    let ports: BTreeMap<&str, &Port> = facts.ports.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut out: BTreeMap<(String, String), BTreeMap<usize, usize>> = BTreeMap::new();
    for link in facts.links.iter().filter(|l| l.kind != LinkKind::Tests) {
        let Some(from) = items.get(link.from.as_str()).filter(|i| !is_test_code(i)) else {
            continue;
        };
        let Some(place) = placements.of(from) else {
            continue;
        };
        let Some(end) = far_end(link, &items, &externals, &ports, placements) else {
            continue;
        };
        if end.area == place.area {
            continue;
        }
        // Keyed by schema position: `LinkKind` has no order of its own.
        let rank = LinkKind::ALL
            .iter()
            .position(|k| *k == link.kind)
            .unwrap_or(0);
        *out.entry((place.area.clone(), end.area))
            .or_default()
            .entry(rank)
            .or_default() += 1;
    }
    out
}

fn rules(out: &mut String, rules: &Rules) {
    out.push_str("## Rules\n\n");
    if rules.rules.is_empty() {
        out.push_str("No dependency rules.\n\n");
        return;
    }
    out.push_str("A subject or a target is a side or an area name; `externals` is everything outside the unit except libraries.\n\n");
    for r in &rules.rules {
        let _ = writeln!(
            out,
            "- {} must not {} {} · {}",
            r.subject,
            r.must_not,
            r.targets.join(", "),
            level_name(r.level)
        );
    }
    out.push('\n');
}

fn findings(out: &mut String, all: &[Finding]) {
    out.push_str("## Open findings\n\n");
    let open: Vec<_> = all.iter().filter(|f| f.allowed.is_none()).collect();
    if open.is_empty() {
        out.push_str("None.\n");
    }
    for f in &open {
        let member = f
            .member
            .as_deref()
            .map_or(String::new(), |m| format!(" (`{m}`)"));
        let _ = writeln!(
            out,
            "- {} · {} · `{}` → `{}`{member} in `{}` · {} · {}",
            level_name(f.level),
            f.rule,
            f.site,
            f.target,
            f.target_area,
            kind_name(f.kind),
            witness(&f.witness)
        );
    }
    let allowed = all.len() - open.len();
    if allowed > 0 {
        let _ = writeln!(
            out,
            "\n{allowed} more covered by `.arch/allows`, not listed."
        );
    }
    out.push('\n');
}

fn descriptions(out: &mut String, input: &PackInput<'_>) {
    out.push_str("## Area descriptions\n\n");
    if input.descriptions.is_empty() {
        out.push_str("None written (`.arch/areas/<name>.md`).\n\n");
        return;
    }
    for (name, text) in input.descriptions {
        let _ = writeln!(out, "### {name}\n\n{}\n", text.trim_end());
    }
}

fn element_section(
    out: &mut String,
    input: &PackInput<'_>,
    placements: &Placements,
    element: &Element,
) {
    let _ = writeln!(
        out,
        "## Your element: {} · `{}`\n",
        element.label, element.id
    );
    let _ = writeln!(out, "Intention: {}\n", element.intention);
    let _ = writeln!(
        out,
        "- site: `{}` · {}",
        element.site,
        position(input, placements, &element.site)
    );
    if let Some(group) = input.plan.group_of(&element.id) {
        let others: Vec<_> = group
            .elements
            .iter()
            .filter(|id| **id != element.id)
            .filter_map(|id| input.plan.element(id))
            .map(|e| e.label.as_str())
            .collect();
        let with = if others.is_empty() {
            "alone".to_string()
        } else {
            format!("with {}", others.join(", "))
        };
        let _ = writeln!(out, "- group: {} ({with})", group.name);
    }
    if !element.depends_on.is_empty() {
        let deps: Vec<_> = element
            .depends_on
            .iter()
            .map(|id| input.plan.element(id).map_or(id.to_string(), label_line))
            .collect();
        let _ = writeln!(out, "- after: {}", deps.join("; "));
    }

    out.push_str("\n### Files you may write\n\n");
    if element.files.is_empty() {
        out.push_str("None: this element writes no file.\n");
    }
    for file in &element.files {
        let file = file.to_string_lossy().replace('\\', "/");
        let _ = writeln!(out, "- `{file}` · {}", position(input, placements, &file));
    }

    out.push_str("\n### What you may depend on\n\n");
    let mut homes: Vec<&Placement> = Vec::new();
    for site in std::iter::once(element.site.clone()).chain(
        element
            .files
            .iter()
            .map(|f| f.to_string_lossy().replace('\\', "/")),
    ) {
        if let Some(p) = place_of(input, placements, &site)
            && !homes.iter().any(|h| h.area == p.area)
        {
            homes.push(p);
        }
    }
    if homes.is_empty() {
        out.push_str("Your site and files are outside every area: no rule applies to them.\n");
    }
    for home in homes {
        reach_line(out, input, home);
    }
}

/// From one area: which areas (and `externals`) the rules let it depend on, and which not.
fn reach_line(out: &mut String, input: &PackInput<'_>, home: &Placement) {
    let home_side = home.side.map(side_name);
    let named = |n: &str| n == home.area || Some(n) == home_side;
    let forbidden: Vec<_> = input
        .rules
        .rules
        .iter()
        .filter(|r| r.must_not == "depend-on" && named(&r.subject))
        .collect();
    let blocked_by = |area: &str, side: Option<&str>| {
        forbidden.iter().position(|r| {
            r.targets
                .iter()
                .any(|t| t == area || Some(t.as_str()) == side)
        })
    };
    let mut may = Vec::new();
    // Areas each rule keeps out of reach, in rule order.
    let mut may_not: Vec<Vec<&str>> = vec![Vec::new(); forbidden.len()];
    let others = input
        .areas
        .areas
        .iter()
        .filter(|a| a.name != home.area)
        .map(|a| (a.name.as_str(), a.side.map(side_name)))
        .chain(std::iter::once((EXTERNALS, Some(EXTERNALS))));
    for (area, side) in others {
        match blocked_by(area, side) {
            Some(rule) => may_not[rule].push(area),
            None => may.push(area.to_string()),
        }
    }
    let may_not: Vec<String> = forbidden
        .iter()
        .zip(&may_not)
        .filter(|(_, areas)| !areas.is_empty())
        .map(|(r, areas)| {
            format!(
                "{} (`{} must not {} {}`)",
                areas.join(", "),
                r.subject,
                r.must_not,
                r.targets.join(", ")
            )
        })
        .collect();
    let side = home_side.unwrap_or("no side");
    let _ = writeln!(out, "From `{}` ({side}):", home.area);
    let _ = writeln!(out, "- may depend on: {}", list_or_none(&may));
    let _ = writeln!(out, "- must not depend on: {}", may_not_or_none(&may_not));
}

fn plan_section(out: &mut String, input: &PackInput<'_>, placements: &Placements) {
    let plan = input.plan;
    let _ = writeln!(out, "## The plan\n\nIntention: {}\n", plan.intention);
    for group in &plan.groups {
        let gate = &group.gate;
        let mut parts: Vec<String> = gate.commands.iter().map(|c| format!("`{c}`")).collect();
        if gate.check {
            parts.push("`arch check`".into());
        }
        if gate.judge {
            parts.push("the judge".into());
        }
        let _ = writeln!(
            out,
            "### Group {} · gate: {}\n",
            group.name,
            list_or_none(&parts)
        );
        for id in &group.elements {
            match plan.element(id) {
                Some(e) => element_line(out, input, placements, e),
                None => {
                    let _ = writeln!(out, "- `{id}` · not in the plan");
                }
            }
        }
        out.push('\n');
    }
    let grouped: BTreeSet<&ElementId> = plan.groups.iter().flat_map(|g| &g.elements).collect();
    let loose: Vec<_> = plan
        .elements
        .iter()
        .filter(|e| !grouped.contains(&e.id))
        .collect();
    if !loose.is_empty() {
        out.push_str("### Not in a group\n\n");
        for e in loose {
            element_line(out, input, placements, e);
        }
        out.push('\n');
    }
}

fn element_line(out: &mut String, input: &PackInput<'_>, placements: &Placements, e: &Element) {
    let _ = writeln!(
        out,
        "- {} · site `{}` · {}",
        label_line(e),
        e.site,
        position(input, placements, &e.site)
    );
    if !e.files.is_empty() {
        let files: Vec<_> = e
            .files
            .iter()
            .map(|f| format!("`{}`", f.to_string_lossy().replace('\\', "/")))
            .collect();
        let _ = writeln!(out, "  - files: {}", files.join(", "));
    }
    if !e.depends_on.is_empty() {
        let deps: Vec<_> = e
            .depends_on
            .iter()
            .map(|id| {
                input
                    .plan
                    .element(id)
                    .map_or(id.to_string(), |d| d.label.clone())
            })
            .collect();
        let _ = writeln!(out, "  - after: {}", deps.join(", "));
    }
}

fn label_line(e: &Element) -> String {
    format!("{} `{}` · {}", e.label, e.id, e.intention)
}

/// Where a site sits: an item id, a repository-relative file, or an area name.
fn place_of<'p>(
    input: &PackInput<'_>,
    placements: &'p Placements,
    site: &str,
) -> Option<&'p Placement> {
    if let Some(item) = input.facts.items.iter().find(|i| i.id == site) {
        return placements.of(item);
    }
    if input.areas.area(site).is_some() {
        return placements.by_area(site);
    }
    placements.of_file(site)
}

fn position(input: &PackInput<'_>, placements: &Placements, site: &str) -> String {
    match place_of(input, placements, site) {
        Some(p) => format!(
            "area `{}` ({})",
            p.area,
            p.side.map_or("no side", side_name)
        ),
        None => "outside every area".to_string(),
    }
}

fn may_not_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join("; ")
    }
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

fn column_rank(c: Column) -> u8 {
    match c {
        Column::Driving | Column::PublicApi => 0,
        Column::Domain | Column::Internals => 1,
        Column::Driven | Column::Leaves => 2,
        Column::Externals => 3,
    }
}

fn level_name(level: arch_facts::Level) -> &'static str {
    match level {
        arch_facts::Level::Block => "block",
        arch_facts::Level::Warn => "warn",
    }
}

fn framework_name(f: &Framework) -> String {
    match f {
        Framework::Generic(dep) => format!("generic, through `{dep}`"),
        other => kebab(&format!("{other:?}")),
    }
}

/// The schema name of a link kind (`calls-port`).
fn kind_name(kind: LinkKind) -> String {
    kebab(&format!("{kind:?}"))
}

fn witness(w: &Witness) -> String {
    match w {
        Witness::Span { file, line, .. } | Witness::Declared { file, line } => {
            format!("`{file}:{line}`")
        }
        Witness::Tool { tool, .. } => format!("`{tool}`"),
    }
}

/// `CallsPort` → `calls-port`, the serialized form of the model's unit enums.
fn kebab(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len() + 4);
    for (i, c) in debug.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use arch_facts::{
        Analyzer, AreaOverride, Confidence, ItemFlags, ItemKind, Level, Link, LinkFlags, Repo,
        Rule, SessionId, Span, Target, Unit, Visibility,
    };

    use super::*;

    fn facts() -> Facts {
        let item = |id: &str, file: &str| Item {
            id: id.into(),
            kind: ItemKind::Fn,
            name: id.into(),
            file: file.into(),
            span: Span {
                line: 1,
                col: 1,
                end_line: 1,
                end_col: 1,
            },
            crate_name: "svc".into(),
            module: String::new(),
            parent: None,
            visibility: Visibility::Pub,
            reexported: false,
            flags: ItemFlags::default(),
            scope: "unit:svc".into(),
        };
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
                name: "hand".into(),
                version: "0".into(),
                degraded: vec![],
            },
        );
        f.items = vec![
            item("svc::app::run#fn", "src/app/mod.rs"),
            item("svc::db::save#fn", "src/db/mod.rs"),
            item("svc::main#fn", "src/main.rs"),
        ];
        f.links = vec![Link {
            from: "svc::app::run#fn".into(),
            to: Target::Item("svc::db::save#fn".into()),
            member: None,
            kind: LinkKind::Calls,
            confidence: Confidence::Resolved,
            witness: Witness::Span {
                file: "src/app/mod.rs".into(),
                line: 3,
                col: None,
            },
            flags: LinkFlags::default(),
            reason: None,
        }];
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
            rule: Some(ColumnRule::Hexagon),
            main_bin: None,
            areas: vec![
                area("db", Column::Driven, "src/db/**"),
                area("app", Column::Domain, "src/app/**"),
            ],
        }
    }

    fn rules() -> Rules {
        Rules {
            rules: vec![Rule {
                subject: "domain".into(),
                must_not: "depend-on".into(),
                targets: vec!["driven".into()],
                level: Level::Block,
            }],
            ..Rules::default()
        }
    }

    fn plan() -> Plan {
        let mut plan = Plan::new(SessionId::new("s1"), "save on run");
        let id = plan.add_element("call save", "svc::app::run#fn").id.clone();
        plan.elements[0].files = vec![PathBuf::from("src/app/mod.rs")];
        plan.groups = vec![arch_facts::Group {
            name: "g1".into(),
            elements: vec![id],
            gate: Default::default(),
        }];
        plan
    }

    fn render(element: Option<&ElementId>, descriptions: &BTreeMap<String, String>) -> String {
        let (facts, areas, rules, plan) = (facts(), areas(), rules(), plan());
        context_pack(&PackInput {
            facts: &facts,
            areas: &areas,
            rules: &rules,
            allows: &Allows::default(),
            descriptions,
            plan: &plan,
            element,
        })
        .unwrap()
    }

    #[test]
    fn the_element_sees_its_position_and_what_the_rules_forbid() {
        let id = plan().elements[0].id.clone();
        let pack = render(Some(&id), &BTreeMap::new());
        assert!(
            pack.contains("- site: `svc::app::run#fn` · area `app` (domain)"),
            "{pack}"
        );
        assert!(
            pack.contains("- `src/app/mod.rs` · area `app` (domain)"),
            "{pack}"
        );
        assert!(
            pack.contains("- must not depend on: db (`domain must not depend-on driven`)"),
            "{pack}"
        );
        assert!(pack.contains("- may depend on: externals"), "{pack}");
        assert!(!pack.contains("## The plan"), "{pack}");
    }

    #[test]
    fn the_judge_sees_the_plan_and_no_element_section() {
        let pack = render(None, &BTreeMap::new());
        assert!(pack.contains("## The plan"), "{pack}");
        assert!(
            pack.contains("### Group g1 · gate: `arch check`, the judge"),
            "{pack}"
        );
        assert!(!pack.contains("## Your element"), "{pack}");
    }

    #[test]
    fn map_counts_cross_area_links_and_findings_stay_open_without_an_allow() {
        let pack = render(None, &BTreeMap::new());
        assert!(pack.contains("- app → db: calls 1"), "{pack}");
        assert!(
            pack.contains("- block · domain must not depend-on driven · `src/app/mod.rs::app::run` → `src/db/mod.rs::db::save` in `db` · calls · `src/app/mod.rs:3`"),
            "{pack}"
        );
        assert!(
            pack.contains("- outside every area (no rule applies): `src/main.rs`"),
            "{pack}"
        );
        // Driving before domain before driven, whatever the file order.
        let (app, db) = (
            pack.find("- **app**").unwrap(),
            pack.find("- **db**").unwrap(),
        );
        assert!(app < db, "{pack}");
    }

    #[test]
    fn area_descriptions_are_carried_verbatim() {
        let descriptions = BTreeMap::from([("app".to_string(), "The use cases.\n".to_string())]);
        let pack = render(None, &descriptions);
        assert!(pack.contains("### app\n\nThe use cases.\n"), "{pack}");
    }

    #[test]
    fn an_element_outside_the_plan_is_an_error() {
        let (facts, areas, rules, plan) = (facts(), areas(), rules(), plan());
        let err = context_pack(&PackInput {
            facts: &facts,
            areas: &areas,
            rules: &rules,
            allows: &Allows::default(),
            descriptions: &BTreeMap::new(),
            plan: &plan,
            element: Some(&ElementId::from_str_unchecked("deadbeef")),
        })
        .unwrap_err();
        assert!(err.to_string().contains("deadbeef"), "{err}");
    }

    #[test]
    fn kebab_matches_the_serialized_names() {
        assert_eq!(kind_name(LinkKind::CallsPort), "calls-port");
        assert_eq!(kind_name(LinkKind::Calls), "calls");
        assert_eq!(kebab("DataStores"), "data-stores");
    }
}
