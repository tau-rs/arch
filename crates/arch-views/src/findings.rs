//! Findings from dependency rules: `subject must not depend-on targets` evaluated on links.
//!
//! A pure function of the facts and three `.arch/` files (`areas.toml`, `rules`, `allows`).
//!
//! - **What depends on what.** Every link from an item is a dependency of that item's area on the
//!   target's, except `tests` links and links from test code (ADR 0027: non-test code only).
//!   A link to a table, to an I/O external or to a driven port depends on `externals`; libraries
//!   (externals of kind `crate` or `pub`) do not count, or every module would (ADR 0027).
//! - **Who a rule is about.** A subject or target is a side (`domain`, `driven`, …) or an area
//!   name. An item outside every area has no column and no rule applies to it (ADR 0025).
//! - **Confidence** (ADR 0009). A finding on a `guessed` link warns and never blocks; on a
//!   `declared` link it cannot gate either. Only `resolved` links keep the rule's level.
//! - **Allows.** A finding whose site, rule and target match an entry of `.arch/allows` is kept
//!   in the report, marked allowed, and never blocks.
//! - Every finding carries its link's witness and `origin: core` (spec §3).

use std::collections::BTreeMap;

use arch_facts::{
    Allow, Allows, Areas, Confidence, External, Facts, Item, Level, Link, LinkKind, Port, Rule,
    Rules, Side, Target, Witness,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::Error;
use crate::placement::{Placement, Placements, side_name};
use crate::propose::{is_io, is_test_code};

/// The name `.arch/rules` uses for everything outside the unit.
pub(crate) const EXTERNALS: &str = "externals";

/// One rule violation on one link.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Finding {
    /// The rule as `.arch/allows` spells it: `domain must not depend-on driven`.
    pub rule: String,
    /// Who contributed the check; always `core` in V1.
    pub origin: &'static str,
    /// The effective level, after the confidence rule and allows.
    pub level: Level,
    /// The site: `<file>::<path of the item in its module>`, as `.arch/allows` keys it.
    pub site: String,
    /// The item the link starts from.
    pub from: String,
    /// The area of `from`.
    pub from_area: String,
    /// The far end, in the same form as `site` for an item, or the id of an external, port or table.
    pub target: String,
    /// The area of the target, or `externals`.
    pub target_area: String,
    /// The link's kind.
    pub kind: LinkKind,
    /// The variant or field touched, when the link names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// The link's confidence.
    pub confidence: Confidence,
    /// Where the link is visible.
    pub witness: Witness,
    /// Set when `.arch/allows` covers the finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Allowed>,
}

/// The allow that covers a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Allowed {
    /// Who allowed it.
    pub by: String,
    /// Why.
    pub reason: String,
}

impl Finding {
    /// Whether this finding fails a gate: level `block` and not allowed.
    pub fn blocks(&self) -> bool {
        self.level == Level::Block && self.allowed.is_none()
    }
}

/// The findings of one run, sorted by site, target, rule and kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct Report {
    /// Every finding, allowed ones included.
    pub findings: Vec<Finding>,
}

impl Report {
    /// Findings that fail a gate.
    pub fn blocking(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.blocks())
    }

    /// Findings that warn: not allowed and not blocking.
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.allowed.is_none() && !f.blocks())
    }

    /// Findings covered by an allow.
    pub fn allowed(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.allowed.is_some())
    }
}

/// Evaluate the dependency rules of `.arch/rules` on the facts.
pub fn check_rules(
    facts: &Facts,
    areas: &Areas,
    rules: &Rules,
    allows: &Allows,
) -> Result<Report, Error> {
    let placements = Placements::new(areas)?;
    let items: BTreeMap<&str, &Item> = facts.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let externals: BTreeMap<&str, &External> =
        facts.externals.iter().map(|e| (e.id.as_str(), e)).collect();
    let ports: BTreeMap<&str, &Port> = facts.ports.iter().map(|p| (p.id.as_str(), p)).collect();

    let mut findings = Vec::new();
    for link in &facts.links {
        if link.kind == LinkKind::Tests {
            continue;
        }
        let Some(from) = items.get(link.from.as_str()) else {
            continue;
        };
        if is_test_code(from) {
            continue;
        }
        let Some(from_place) = placements.of(from) else {
            continue;
        };
        let Some(end) = far_end(link, &items, &externals, &ports, &placements) else {
            continue;
        };
        if end.area == from_place.area {
            continue;
        }
        for rule in rules.rules.iter().filter(|r| r.must_not == "depend-on") {
            if !names(from_place, &rule.subject) {
                continue;
            }
            let Some(hit) = rule.targets.iter().find(|t| end.is_named(t)) else {
                continue;
            };
            findings.push(finding(rule, hit, link, from, from_place, &end, allows));
        }
    }
    findings.sort_by(|a, b| {
        (
            &a.site,
            &a.target,
            &a.rule,
            a.witness_key(),
            format!("{:?}", a.kind),
        )
            .cmp(&(
                &b.site,
                &b.target,
                &b.rule,
                b.witness_key(),
                format!("{:?}", b.kind),
            ))
    });
    Ok(Report { findings })
}

impl Finding {
    fn witness_key(&self) -> (String, u32) {
        match &self.witness {
            Witness::Span { file, line, .. } | Witness::Declared { file, line } => {
                (file.clone(), *line)
            }
            Witness::Tool { tool, .. } => (tool.clone(), 0),
        }
    }
}

/// The far end of a link, as rules see it.
pub(crate) struct FarEnd {
    /// Display form: an allow-style site for an item, an id otherwise.
    target: String,
    /// The target item's id, when the target is an item.
    item_id: Option<String>,
    /// Area name, or `externals`.
    pub(crate) area: String,
    /// Side name, when the area has one; `externals` for everything outside the unit.
    side: Option<&'static str>,
}

impl FarEnd {
    fn is_named(&self, name: &str) -> bool {
        self.area == name || self.side == Some(name)
    }
}

pub(crate) fn far_end(
    link: &Link,
    items: &BTreeMap<&str, &Item>,
    externals: &BTreeMap<&str, &External>,
    ports: &BTreeMap<&str, &Port>,
    placements: &Placements,
) -> Option<FarEnd> {
    let outside = |id: &str| FarEnd {
        target: id.to_string(),
        item_id: None,
        area: EXTERNALS.to_string(),
        side: Some(EXTERNALS),
    };
    match &link.to {
        Target::Item(id) => {
            let item = items.get(id.as_str())?;
            let place = placements.of(item)?;
            Some(FarEnd {
                target: site_of(item),
                item_id: Some(item.id.clone()),
                area: place.area.clone(),
                side: place.side.map(side_name),
            })
        }
        Target::Table(name) => Some(outside(&format!("table:{name}"))),
        Target::External(id) => {
            let external = externals.get(id.as_str())?;
            is_io(external.kind).then(|| outside(id))
        }
        Target::Port(id) => {
            let port = ports.get(id.as_str())?;
            (port.side == Side::Driven && is_io(port.kind)).then(|| outside(id))
        }
    }
}

fn names(place: &Placement, name: &str) -> bool {
    place.area == name || place.side.map(side_name) == Some(name)
}

fn finding(
    rule: &Rule,
    hit: &str,
    link: &Link,
    from: &Item,
    from_place: &Placement,
    end: &FarEnd,
    allows: &Allows,
) -> Finding {
    let rule_text = format!("{} must not {} {}", rule.subject, rule.must_not, hit);
    let site = site_of(from);
    let allowed = allows
        .allows
        .iter()
        .find(|a| allow_covers(a, &rule_text, &site, from, link, end))
        .map(|a| Allowed {
            by: a.by.clone(),
            reason: a.reason.clone(),
        });
    // ADR 0009: only a resolved link keeps the rule's level.
    let level = match link.confidence {
        Confidence::Resolved => rule.level,
        Confidence::Guessed | Confidence::Declared => Level::Warn,
    };
    Finding {
        rule: rule_text,
        origin: "core",
        level,
        site,
        from: from.id.clone(),
        from_area: from_place.area.clone(),
        target: end.target.clone(),
        target_area: end.area.clone(),
        kind: link.kind,
        member: link.member.clone(),
        confidence: link.confidence,
        witness: link.witness.clone(),
        allowed,
    }
}

/// An allow covers a finding when the rule matches, the site is the item (by site, by id, or by
/// the witness's `file:line`), and the allow's target, when it names one, is the link's far end
/// or a parent of it (`…::LogNotifier` covers `…::LogNotifier::send`).
fn allow_covers(
    allow: &Allow,
    rule: &str,
    site: &str,
    from: &Item,
    link: &Link,
    end: &FarEnd,
) -> bool {
    if allow.rule != rule {
        return false;
    }
    let at_site = allow.site == site
        || allow.site == from.id
        || matches!(&link.witness, Witness::Span { file, line, .. } if allow.site == format!("{file}:{line}"));
    if !at_site {
        return false;
    }
    match &allow.target {
        None => true,
        Some(target) => {
            *target == end.target
                || end.item_id.as_deref() == Some(target.as_str())
                || end
                    .target
                    .strip_prefix(target.as_str())
                    .is_some_and(|rest| rest.starts_with("::"))
        }
    }
}

/// The site of an item as `.arch/allows` keys it: `<file>::<path of the item in its module>`,
/// with `impl` blocks named by their type (`src/app/notify.rs::NotifyCustomer::deliver`).
pub fn site_of(item: &Item) -> String {
    let id = item
        .id
        .rsplit_once('#')
        .map_or(item.id.as_str(), |(path, _)| path);
    let local = if item.module.is_empty() {
        id.split_once("::").map_or(id, |(_, rest)| rest)
    } else {
        let marker = format!("::{}::", item.module);
        id.find(&marker).map_or(id, |at| &id[at + marker.len()..])
    };
    let local = match local.strip_prefix("impl ") {
        Some(rest) => rest.rsplit_once(" for ").map_or(rest, |(_, ty)| ty),
        None => local,
    };
    format!("{}::{}", item.file, local)
}
