//! The invariant harness: one function per invariant, each over the smallest input it needs.
//!
//! This crate does **not** define the shape of `facts.json` or the view files — `arch` owns those and
//! publishes `schemas/facts.schema.json`. It defines only [`Input`], the harness's own minimal model, and a
//! loader seam ([`load_golden`]) that maps a golden directory onto it once arch's schema exists. Until then the
//! harness is runnable (see the unit tests at the bottom) and the golden test reports "awaiting data".
//!
//! Each check carries its testable statement in its doc comment. Numbering follows arch-design
//! `spec/map-invariants.md` (MAP-1…32, CANVAS-1…2); `invariants.md` maps every number to its check. ADR n means
//! arch-design `adr/00nn-*.md`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------------------------
// Input model (harness-owned, minimal)
// ---------------------------------------------------------------------------------------------

pub type ItemId = String;
pub type LinkId = String;
pub type AreaId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnRule {
    Hexagon,
    Layers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Resolved,
    Guessed,
    Declared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Block,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub krate: String,
    /// Column name under the active rule; `None` when the item is not placed (e.g. an external).
    pub column: Option<String>,
    /// On the crate's public surface (pub item, pub use, registered route/rpc/topic).
    pub public: bool,
    /// Content hash of the facts that place this item; equal hashes ⇒ "unchanged" for MAP-1.
    pub content_hash: String,
    /// The area holding the item; `None` outside every area (the crate root, the Unplaced tray).
    pub area: Option<AreaId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    pub from: ItemId,
    pub to: ItemId,
    /// The link kind as the facts name it: `calls`, `implements`, `uses type`, …
    pub kind: String,
    pub confidence: Confidence,
    /// `Some(reason)` when the analyzer could not resolve the target (dyn, spawn, …).
    pub unresolved: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub item: ItemId,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldState {
    pub visible_items: usize,
    pub scale: f64,
    pub areas_folded: bool,
    pub label_px: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayKind {
    Sessions,
    Plan,
    Findings,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Outline,
    Fill,
    Fade,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayRow {
    pub kind: OverlayKind,
    pub item: ItemId,
    pub channel: Channel,
    /// True if the row carries any geometry (x, y, w, h, group box). Must be false.
    pub has_geometry: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pill {
    pub item: ItemId,
    pub kind: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub id: AreaId,
    /// The area's column under the unit's rule, computed or overridden in `.arch/areas.toml`.
    pub side: String,
    /// Drawn as a chip with counts.
    pub folded: bool,
}

/// A link drawn between two folded chips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChipLink {
    pub from: AreaId,
    pub to: AreaId,
    pub count: usize,
    pub badged: bool,
}

/// The Reach set the view shows around a focused item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reach {
    pub focus: ItemId,
    pub depth: usize,
    pub items: Vec<ItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    /// The world calls the unit.
    Driving,
    /// The unit calls the world.
    Driven,
}

/// Rail sections in the map's fixed order; the derived `Ord` is that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    PlatformServices,
    ThirdParty,
    Events,
    DataStores,
    Os,
    Libraries,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub id: String,
    pub kind: String,
    pub side: Side,
    pub section: Section,
    /// The external behind the port.
    pub external: Option<String>,
}

/// An edge from an item to an external, as drawn: it ends on `port`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RailEdge {
    pub from: ItemId,
    pub external: String,
    pub port: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub rule: String,
    pub link: Option<LinkId>,
    pub level: Level,
    pub gates_merge: bool,
    pub origin: String,
}

/// A facts/views pair reduced to what the invariants read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub rule: Option<ColumnRule>,
    /// The unit has an entry: `main`, framework-held or a spawned worker (ADR 0028).
    pub has_entry: bool,
    /// `rule` in `.arch/areas.toml`, overriding the one chosen from the entry.
    pub rule_override: Option<ColumnRule>,
    /// Column names left → right under `rule`.
    pub columns: Vec<String>,
    pub areas: Vec<Area>,
    pub items: Vec<Item>,
    pub links: Vec<Link>,
    /// External ids from the facts.
    pub externals: Vec<String>,
    /// Ports in rail order (top to bottom on each side).
    pub ports: Vec<Port>,
    pub rail_edges: Vec<RailEdge>,
    /// The candidate positions (the view under test).
    pub positions: Vec<Position>,
    /// Positions from the previous save (the golden), for MAP-1.
    pub baseline_positions: Option<Vec<Position>>,
    /// Baseline items, to know which were unchanged.
    pub baseline_items: Option<Vec<Item>>,
    /// Positions computed with every overlay on; must equal `positions` (MAP-3).
    pub positions_with_overlays: Option<Vec<Position>>,
    /// Positions computed with the ports and rail edges left out; must equal `positions` (MAP-32).
    pub positions_without_rails: Option<Vec<Position>>,
    /// Fold states observed while folding, first is the opening state.
    pub fold: Vec<FoldState>,
    pub overlays: Vec<OverlayRow>,
    pub pills: Vec<Pill>,
    pub chip_links: Vec<ChipLink>,
    pub reach: Option<Reach>,
    pub findings: Vec<Finding>,
}

// ---------------------------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub invariant: &'static str,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.invariant, self.message)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub violations: Vec<Violation>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.violations.is_empty() {
            return write!(f, "all invariants hold");
        }
        for v in &self.violations {
            writeln!(f, "{v}")?;
        }
        Ok(())
    }
}

/// Above this many visible items, areas fold to chips (spec §5: "above ~80").
pub const FOLD_ITEM_THRESHOLD: usize = 80;
/// The fold keeps the footprint with a floor at this fraction of the opening scale (MAP-4 amended).
pub const FOLD_SCALE_FLOOR: f64 = 0.7;
/// The link kinds that carry a direction, spelled as the schema spells them (ADR 0025 decision 1, ADR 0033).
pub const DIRECTED_KINDS: [&str; 5] = ["calls", "calls-out", "hands-off", "listens-to", "routes"];
/// The hexagon's item columns; the grain points toward `domain` (ADR 0025 decision 3).
pub const HEXAGON_SIDES: [&str; 3] = ["driving", "domain", "driven"];
/// The hexagon's rail column: it holds ports, never items (MAP-31).
pub const RAIL_COLUMN: &str = "externals";
/// The port kinds MAP-31 lists.
pub const PORT_KINDS: [&str; 10] = ["rpc", "http", "cli", "topic", "crate", "sql", "pub", "redis", "fs", "tty"];

/// Run every check. The only entry point `arch`'s CI needs.
pub fn check_all(input: &Input) -> Report {
    let mut v = Vec::new();
    v.extend(positions_stable(input));
    v.extend(columns_are_sides(input));
    v.extend(no_state_relayout(input));
    v.extend(fold_floor(input));
    v.extend(folded_link_counts(input));
    v.extend(reach_depth(input));
    v.extend(overlay_stacking(input));
    v.extend(unresolved_one_pill(input));
    v.extend(direction_left_to_right(input));
    v.extend(column_rule_from_entry(input));
    v.extend(cross_unit_on_public_surface(input));
    v.extend(externals_on_rails(input));
    v.extend(rail_docking_moves_nothing(input));
    v.extend(ports_from_facts(input));
    v.extend(finding_confidence(input));
    v.extend(origin_is_core(input));
    Report { violations: v }
}

fn viol(invariant: &'static str, message: impl Into<String>) -> Violation {
    Violation { invariant, message: message.into() }
}

fn by_item(positions: &[Position]) -> HashMap<&str, &Position> {
    positions.iter().map(|p| (p.item.as_str(), p)).collect()
}

fn same_coords(a: &Position, b: &Position) -> bool {
    a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()
}

/// Positions computed `with` something must equal the plain positions bit for bit.
fn moved_positions(invariant: &'static str, what: &str, with: &[Position], plain: &[Position]) -> Vec<Violation> {
    let by_plain = by_item(plain);
    let mut out = Vec::new();
    for p in with {
        match by_plain.get(p.item.as_str()) {
            Some(q) if same_coords(p, q) => {}
            Some(q) => out.push(viol(invariant, format!("{} is at ({}, {}) with {what} but ({}, {}) without", p.item, p.x, p.y, q.x, q.y))),
            None => out.push(viol(invariant, format!("{} appears only with {what}", p.item))),
        }
    }
    if with.len() != plain.len() {
        out.push(viol(invariant, format!("{} positions with {what}, {} without", with.len(), plain.len())));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------------------------

/// MAP-1 / MAP-24 · Positions inside a unit come from facts and are byte-identical between saves for unchanged
/// items. Testable: for every item present in baseline and candidate with the same content hash, the
/// candidate position equals the baseline position bit for bit. Skipped when no baseline is given.
pub fn positions_stable(input: &Input) -> Vec<Violation> {
    let (Some(base_pos), Some(base_items)) = (&input.baseline_positions, &input.baseline_items) else {
        return Vec::new();
    };
    let base_hash: HashMap<&str, &str> = base_items.iter().map(|i| (i.id.as_str(), i.content_hash.as_str())).collect();
    let base = by_item(base_pos);
    let cand = by_item(&input.positions);
    let mut out = Vec::new();
    for item in &input.items {
        if base_hash.get(item.id.as_str()) != Some(&item.content_hash.as_str()) {
            continue;
        }
        match (base.get(item.id.as_str()), cand.get(item.id.as_str())) {
            (Some(b), Some(c)) if !same_coords(b, c) => {
                out.push(viol("MAP-1", format!("{} moved from ({}, {}) to ({}, {}) with unchanged facts", item.id, b.x, b.y, c.x, c.y)));
            }
            (Some(_), None) => out.push(viol("MAP-1", format!("{} lost its position with unchanged facts", item.id))),
            _ => {}
        }
    }
    out
}

/// MAP-3 · The map never re-layouts for state. Testable: positions with every overlay on equal the
/// positions with none; no overlay row carries geometry.
pub fn no_state_relayout(input: &Input) -> Vec<Violation> {
    let mut out = match &input.positions_with_overlays {
        Some(with) => moved_positions("MAP-3", "overlays", with, &input.positions),
        None => Vec::new(),
    };
    for row in input.overlays.iter().filter(|r| r.has_geometry) {
        out.push(viol("MAP-3", format!("{:?} overlay row on {} carries geometry", row.kind, row.item)));
    }
    out
}

/// MAP-4 (amended) · Fold, never shrink. Testable: one label size across every fold state; every state's
/// scale ≥ 0.7 × the opening scale; a state with more than ~80 visible items has its areas folded.
pub fn fold_floor(input: &Input) -> Vec<Violation> {
    let Some(opening) = input.fold.first() else { return Vec::new() };
    let mut out = Vec::new();
    for (i, s) in input.fold.iter().enumerate() {
        if s.label_px.to_bits() != opening.label_px.to_bits() {
            out.push(viol("MAP-4", format!("fold state {i} changes label size {} → {}", opening.label_px, s.label_px)));
        }
        if s.scale < FOLD_SCALE_FLOOR * opening.scale {
            out.push(viol("MAP-4", format!("fold state {i} scale {} is below the floor {} × {}", s.scale, FOLD_SCALE_FLOOR, opening.scale)));
        }
        if s.visible_items > FOLD_ITEM_THRESHOLD && !s.areas_folded {
            out.push(viol("MAP-4", format!("fold state {i} shows {} items with areas unfolded", s.visible_items)));
        }
    }
    out
}

/// MAP-14 · Overlay stacking: outline = session, fill = plan or finding, delta fades; no overlay groups
/// items spatially. Testable: every overlay row's channel matches its kind.
pub fn overlay_stacking(input: &Input) -> Vec<Violation> {
    input
        .overlays
        .iter()
        .filter_map(|r| {
            let expected = match r.kind {
                OverlayKind::Sessions => Channel::Outline,
                OverlayKind::Plan | OverlayKind::Findings => Channel::Fill,
                OverlayKind::Delta => Channel::Fade,
            };
            (r.channel != expected).then(|| viol("MAP-14", format!("{:?} overlay on {} uses {:?}, expected {:?}", r.kind, r.item, r.channel, expected)))
        })
        .collect()
}

/// MAP-16 / MAP-28 · Unresolved paths are folded to one pill per item. Testable: an item with n > 0
/// unresolved outgoing links has exactly one `unresolved` pill with count n; an item with none has no such pill.
pub fn unresolved_one_pill(input: &Input) -> Vec<Violation> {
    let mut unresolved: BTreeMap<&str, usize> = BTreeMap::new();
    for l in input.links.iter().filter(|l| l.unresolved.is_some()) {
        *unresolved.entry(l.from.as_str()).or_default() += 1;
    }
    let mut pills: HashMap<&str, Vec<&Pill>> = HashMap::new();
    for p in input.pills.iter().filter(|p| p.kind == "unresolved") {
        pills.entry(p.item.as_str()).or_default().push(p);
    }
    let mut out = Vec::new();
    for item in &input.items {
        let n = unresolved.get(item.id.as_str()).copied().unwrap_or(0);
        let ps = pills.get(item.id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        match (n, ps) {
            (0, []) => {}
            (0, _) => out.push(viol("MAP-16", format!("{} has an unresolved pill but no unresolved links", item.id))),
            (_, [p]) if p.count == n => {}
            (_, [p]) => out.push(viol("MAP-16", format!("{} pill says {} unresolved, facts say {}", item.id, p.count, n))),
            (_, ps) => out.push(viol("MAP-16", format!("{} has {} unresolved pills, expected one", item.id, ps.len()))),
        }
    }
    out
}

/// MAP-30 (V2, vacuous with one unit) · Cross-unit links land on public surfaces (spec §3, §7). Testable: for every link whose endpoints are in
/// different crates, the target item is on its crate's public surface. V1 has one unit; crates stand in.
pub fn cross_unit_on_public_surface(input: &Input) -> Vec<Violation> {
    let items: HashMap<&str, &Item> = input.items.iter().map(|i| (i.id.as_str(), i)).collect();
    input
        .links
        .iter()
        .filter(|l| l.unresolved.is_none())
        .filter_map(|l| {
            let (from, to) = (items.get(l.from.as_str())?, items.get(l.to.as_str())?);
            (from.krate != to.krate && !to.public)
                .then(|| viol("MAP-30", format!("link {} crosses {} → {} onto non-public {}", l.id, from.krate, to.krate, to.id)))
        })
        .collect()
}

/// MAP-17 (ADR 0025) · A directed link against the grain of the unit's column rule is drawn dashed as a
/// smell; no other link kind carries a direction. Testable: for every resolved link of a kind in
/// [`DIRECTED_KINDS`] between two items that both have an item column, if the link runs against the grain a
/// `direction` finding names it. Layers: against = to a left-hand column. Hexagon: with the grain = driving →
/// domain or driven → domain; any other move between two of driving · domain · driven is against. A link
/// inside one column, or with an endpoint outside the item columns (the rail, the crate root), has no direction.
pub fn direction_left_to_right(input: &Input) -> Vec<Violation> {
    let Some(rule) = input.rule else { return Vec::new() };
    let col_index: HashMap<&str, usize> = input.columns.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
    let items: HashMap<&str, &Item> = input.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let smells: HashMap<&str, &Finding> = input.findings.iter().filter(|f| f.rule == "direction").filter_map(|f| Some((f.link.as_deref()?, f))).collect();
    let item_column = |id: &str| -> Option<&str> {
        let c = items.get(id)?.column.as_deref()?;
        let placed = match rule {
            ColumnRule::Layers => col_index.contains_key(c),
            ColumnRule::Hexagon => HEXAGON_SIDES.contains(&c) && col_index.contains_key(c),
        };
        placed.then_some(c)
    };
    let against = |from: &str, to: &str| match rule {
        ColumnRule::Layers => col_index[to] < col_index[from],
        ColumnRule::Hexagon => from != to && !(to == "domain" && (from == "driving" || from == "driven")),
    };
    let mut out = Vec::new();
    for l in input.links.iter().filter(|l| l.unresolved.is_none() && DIRECTED_KINDS.contains(&l.kind.as_str())) {
        let (Some(cf), Some(ct)) = (item_column(&l.from), item_column(&l.to)) else { continue };
        if against(cf, ct) && !smells.contains_key(l.id.as_str()) {
            out.push(viol("MAP-17", format!("{} link {} goes {cf} → {ct}, against the {rule:?} grain, and no direction finding marks it", l.kind, l.id)));
        }
    }
    out
}

/// MAP-2 (and MAP-19's fixtures half) · Columns are sides, computed; an area cannot change column. Testable:
/// every area's side is a column under the unit's rule, and every item in an area sits in that area's column,
/// so an override that spans two sides is rejected. Items outside every area are not checked.
pub fn columns_are_sides(input: &Input) -> Vec<Violation> {
    if input.rule.is_none() {
        return Vec::new();
    }
    let areas: HashMap<&str, &Area> = input.areas.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut out = Vec::new();
    for a in input.areas.iter().filter(|a| !input.columns.contains(&a.side)) {
        out.push(viol("MAP-2", format!("area {} has side {:?}, not a column under {:?}", a.id, a.side, input.rule.unwrap())));
    }
    for item in &input.items {
        let Some(area_id) = item.area.as_deref() else { continue };
        match areas.get(area_id) {
            None => out.push(viol("MAP-2", format!("{} is in unknown area {area_id}", item.id))),
            Some(a) if item.column.as_deref() != Some(a.side.as_str()) => {
                out.push(viol("MAP-2", format!("{} sits in column {:?} but its area {} is on side {:?}", item.id, item.column, a.id, a.side)));
            }
            Some(_) => {}
        }
    }
    out
}

/// MAP-7 · Links between folded chips carry counts equal to the item links they fold; a finding on a folded
/// link badges the chip. Testable, under the reading proposed in tau-rs/arch-design#120: between two folded
/// areas A ≠ B there is exactly one chip link A → B per direction that folds any link, its count equals the
/// resolved item → item links (any kind) from A's items to B's items, and it is badged iff a finding names one
/// of them. Unresolved links stay on their item's pill (MAP-16).
pub fn folded_link_counts(input: &Input) -> Vec<Violation> {
    let folded: HashMap<&str, &Area> = input.areas.iter().filter(|a| a.folded).map(|a| (a.id.as_str(), a)).collect();
    let area_of: HashMap<&str, &str> = input.items.iter().filter_map(|i| Some((i.id.as_str(), i.area.as_deref()?))).collect();
    let with_finding: std::collections::HashSet<&str> = input.findings.iter().filter_map(|f| f.link.as_deref()).collect();
    let mut expected: BTreeMap<(&str, &str), (usize, bool)> = BTreeMap::new();
    for l in input.links.iter().filter(|l| l.unresolved.is_none()) {
        let (Some(&a), Some(&b)) = (area_of.get(l.from.as_str()), area_of.get(l.to.as_str())) else { continue };
        if a != b && folded.contains_key(a) && folded.contains_key(b) {
            let e = expected.entry((a, b)).or_default();
            e.0 += 1;
            e.1 |= with_finding.contains(l.id.as_str());
        }
    }
    let mut drawn: BTreeMap<(&str, &str), Vec<&ChipLink>> = BTreeMap::new();
    for c in &input.chip_links {
        drawn.entry((c.from.as_str(), c.to.as_str())).or_default().push(c);
    }
    let mut out = Vec::new();
    for (&(a, b), &(n, badge)) in &expected {
        match drawn.get(&(a, b)).map(Vec::as_slice).unwrap_or(&[]) {
            [] => out.push(viol("MAP-7", format!("chips {a} → {b} fold {n} links but no chip link is drawn"))),
            [c] if c.count != n => out.push(viol("MAP-7", format!("chip link {a} → {b} says {}, it folds {n} links", c.count))),
            [c] if c.badged != badge => out.push(viol("MAP-7", format!("chip link {a} → {b} badge is {}, findings on its links say {badge}", c.badged))),
            [_] => {}
            cs => out.push(viol("MAP-7", format!("{} chip links drawn {a} → {b}, expected one", cs.len()))),
        }
    }
    for &(a, b) in drawn.keys().filter(|k| !expected.contains_key(*k)) {
        out.push(viol("MAP-7", format!("chip link {a} → {b} folds no item link between folded areas")));
    }
    out
}

/// MAP-11 · Focus fades everything outside the Reach ring; the ring's depth comes from the Reach stepper.
/// Testable (the set, not the fade), under the reading proposed in tau-rs/arch-design#119: the Reach set at
/// depth n is the focus plus every item within n hops over resolved item → item links of any kind, followed in
/// both directions; unresolved paths add no items.
pub fn reach_depth(input: &Input) -> Vec<Violation> {
    let Some(reach) = &input.reach else { return Vec::new() };
    if !input.items.iter().any(|i| i.id == reach.focus) {
        return vec![viol("MAP-11", format!("Reach focus {} is not an item", reach.focus))];
    }
    let mut neighbours: HashMap<&str, Vec<&str>> = HashMap::new();
    for l in input.links.iter().filter(|l| l.unresolved.is_none()) {
        neighbours.entry(l.from.as_str()).or_default().push(l.to.as_str());
        neighbours.entry(l.to.as_str()).or_default().push(l.from.as_str());
    }
    let mut expected: std::collections::BTreeSet<&str> = [reach.focus.as_str()].into();
    let mut frontier = vec![reach.focus.as_str()];
    for _ in 0..reach.depth {
        frontier = frontier.iter().flat_map(|i| neighbours.get(i).into_iter().flatten().copied()).filter(|n| expected.insert(n)).collect();
    }
    let shown: std::collections::BTreeSet<&str> = reach.items.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for missing in expected.difference(&shown) {
        out.push(viol("MAP-11", format!("{missing} is within {} hops of {} but outside the Reach set", reach.depth, reach.focus)));
    }
    for extra in shown.difference(&expected) {
        out.push(viol("MAP-11", format!("{extra} is in the Reach set but more than {} hops from {}", reach.depth, reach.focus)));
    }
    out
}

/// MAP-26 · Two column rules, hexagon (the unit has an entry) and layers (no entry), chosen per unit,
/// overridable in `.arch/areas.toml`. Testable: the unit's rule is the override when one is set, else hexagon
/// with an entry and layers without.
pub fn column_rule_from_entry(input: &Input) -> Vec<Violation> {
    let Some(rule) = input.rule else { return Vec::new() };
    let expected = input.rule_override.unwrap_or(if input.has_entry { ColumnRule::Hexagon } else { ColumnRule::Layers });
    if rule == expected {
        return Vec::new();
    }
    let why = match input.rule_override {
        Some(_) => "the areas.toml override".to_string(),
        None => format!("has_entry = {}", input.has_entry),
    };
    vec![viol("MAP-26", format!("unit rule is {rule:?}, {why} says {expected:?}"))]
}

/// MAP-31 · Externals live at the interface tier only: every external is a port of kind rpc · http · cli ·
/// topic · crate · sql · pub · redis · fs · tty, declared on the unit's flat sides in the fixed section order
/// platform services · third-party · events · data stores · os · libraries · unresolved; there is no outbound
/// column. Testable as written: every port's kind is in [`PORT_KINDS`]; every external is some port's
/// external; on each side the ports' sections never go back in the fixed order; no external is placed as a
/// column item and no item sits in the rail column. arch's facts disagree today (driving ports have no
/// external, crate externals have no port): tau-rs/arch-design#118.
pub fn externals_on_rails(input: &Input) -> Vec<Violation> {
    let mut out = Vec::new();
    for p in input.ports.iter().filter(|p| !PORT_KINDS.contains(&p.kind.as_str())) {
        out.push(viol("MAP-31", format!("port {} has kind {:?}, not one of MAP-31's", p.id, p.kind)));
    }
    for e in input.externals.iter().filter(|e| !input.ports.iter().any(|p| p.external.as_ref() == Some(*e))) {
        out.push(viol("MAP-31", format!("external {e} is not a port")));
    }
    for side in [Side::Driving, Side::Driven] {
        let on_side: Vec<&Port> = input.ports.iter().filter(|p| p.side == side).collect();
        for w in on_side.windows(2).filter(|w| w[1].section < w[0].section) {
            out.push(viol("MAP-31", format!("{side:?} rail puts {} ({:?}) after {} ({:?})", w[1].id, w[1].section, w[0].id, w[0].section)));
        }
    }
    for i in &input.items {
        if i.column.as_deref() == Some(RAIL_COLUMN) {
            out.push(viol("MAP-31", format!("{} sits in the rail column {RAIL_COLUMN}", i.id)));
        } else if i.column.is_some() && input.externals.contains(&i.id) {
            out.push(viol("MAP-31", format!("external {} is placed in column {:?}", i.id, i.column)));
        }
    }
    out
}

/// MAP-32 · Edges dock to ports and detour rather than move anything. Testable (the fixtures half): positions
/// computed with the ports and rail edges equal the positions computed without them, bit for bit. "A
/// neighbour's matching port is wired straight" needs a second unit (V2); the rail's unresolved section is a
/// sett story.
pub fn rail_docking_moves_nothing(input: &Input) -> Vec<Violation> {
    match &input.positions_without_rails {
        Some(without) => moved_positions("MAP-32", "rails", &input.positions, without),
        None => Vec::new(),
    }
}

/// CANVAS-2 · Ports come from facts (every port has a witness external); an edge to an external ends on that
/// external's port. Testable as written: every port names an external present in the facts, and every rail
/// edge ends on a port whose external is the edge's external. arch's facts give driving ports no external:
/// tau-rs/arch-design#118.
pub fn ports_from_facts(input: &Input) -> Vec<Violation> {
    let ports: HashMap<&str, &Port> = input.ports.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut out = Vec::new();
    for p in &input.ports {
        match &p.external {
            None => out.push(viol("CANVAS-2", format!("port {} has no witness external", p.id))),
            Some(e) if !input.externals.contains(e) => out.push(viol("CANVAS-2", format!("port {} names external {e}, absent from the facts", p.id))),
            Some(_) => {}
        }
    }
    for edge in &input.rail_edges {
        let docked = edge.port.as_deref().and_then(|id| ports.get(id));
        if docked.and_then(|p| p.external.as_ref()) != Some(&edge.external) {
            out.push(viol("CANVAS-2", format!("edge {} → {} ends on {:?}, not on that external's port", edge.from, edge.external, edge.port)));
        }
    }
    out
}

/// ADR 9 · Confidence rule. Testable: a finding on a `guessed` link warns and never blocks; a finding on a
/// `declared` link cannot gate a merge.
pub fn finding_confidence(input: &Input) -> Vec<Violation> {
    let links: HashMap<&str, &Link> = input.links.iter().map(|l| (l.id.as_str(), l)).collect();
    let mut out = Vec::new();
    for f in &input.findings {
        let Some(link) = f.link.as_deref().and_then(|id| links.get(id)) else { continue };
        match link.confidence {
            Confidence::Guessed if f.level == Level::Block || f.gates_merge => {
                out.push(viol("ADR-9", format!("finding {} on guessed link {} blocks or gates", f.id, link.id)));
            }
            Confidence::Declared if f.gates_merge => out.push(viol("ADR-9", format!("finding {} on declared link {} gates a merge", f.id, link.id))),
            _ => {}
        }
    }
    out
}

/// Spec §3 · In V1 every finding's origin is `core`. Testable as written.
pub fn origin_is_core(input: &Input) -> Vec<Violation> {
    input.findings.iter().filter(|f| f.origin != "core").map(|f| viol("ORIGIN", format!("finding {} has origin {:?}", f.id, f.origin))).collect()
}

// ---------------------------------------------------------------------------------------------
// Loader seam
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub enum LoadError {
    /// No `facts.json` in the directory: golden data not generated yet.
    AwaitingData,
    /// `facts.json` is present but is not a JSON document the loader can read.
    AwaitingSchema(String),
    Io(std::io::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::AwaitingData => write!(f, "awaiting data: no facts.json (generated by arch-analyze)"),
            LoadError::AwaitingSchema(p) => write!(f, "facts.json is not readable as arch's facts document: {p}"),
            LoadError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Map a `golden/<repo>/` directory (facts.json + views/) onto [`Input`].
///
/// Returns [`LoadError::AwaitingData`] while the golden files do not exist. Once `arch` publishes its facts
/// schema this is the one function to write; the checks above do not change.
pub fn load_golden(dir: &Path) -> Result<Input, LoadError> {
    let facts = dir.join("facts.json");
    if !facts.exists() {
        return Err(LoadError::AwaitingData);
    }
    let text = std::fs::read_to_string(&facts).map_err(LoadError::Io)?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| LoadError::AwaitingSchema(format!("{}: {e}", facts.display())))?;
    Ok(input_from_facts(&doc))
}

/// The part of [`Input`] that `facts.json` alone determines: items and the links between them.
///
/// Columns, positions, fold states, overlays, pills and findings come from `views/`, which
/// `arch-views` does not emit yet; they stay empty, so the checks that read them hold trivially
/// until the view files land (issue 4). Field names follow arch's `schemas/facts.schema.json`.
pub fn input_from_facts(doc: &serde_json::Value) -> Input {
    let list = |key: &str| doc.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let entries: std::collections::BTreeSet<String> = list("entries").iter().map(|e| text(e, "item")).collect();
    let items = list("items")
        .iter()
        .map(|i| {
            let id = text(i, "id");
            let reexported = i.get("reexported").and_then(|x| x.as_bool()).unwrap_or(false);
            Item {
                krate: text(i, "crate"),
                column: None,
                // Areas come from `views/`, not from facts.
                area: None,
                // Public surface: pub items, pub use, registered routes (the framework-held entries).
                public: text(i, "visibility") == "pub" || reexported || entries.contains(&id),
                // The facts that place an item are the item's own; its serialization stands for them.
                content_hash: i.to_string(),
                id,
            }
        })
        .collect();
    let links = list("links")
        .iter()
        .filter_map(|l| {
            // Only item → item links are drawn between placed items; ports, externals and tables are rails.
            let to = l.get("to")?.get("item")?.as_str()?.to_string();
            let (from, kind) = (text(l, "from"), text(l, "kind"));
            let member = l.get("member").and_then(|m| m.as_str()).map(|m| format!(".{m}")).unwrap_or_default();
            let confidence = match text(l, "confidence").as_str() {
                "resolved" => Confidence::Resolved,
                "declared" => Confidence::Declared,
                _ => Confidence::Guessed,
            };
            Some(Link { id: format!("{from} -{kind}{member}-> {to}"), from, to, kind, confidence, unresolved: None })
        })
        .collect();
    Input { items, links, ..Input::default() }
}

// ---------------------------------------------------------------------------------------------
// Unit tests: the harness is runnable now, on hand-built inputs. These are not golden data.
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, krate: &str, column: &str, public: bool) -> Item {
        Item { id: id.into(), krate: krate.into(), column: Some(column.into()), public, content_hash: format!("h-{id}"), area: None }
    }

    fn link(id: &str, from: &str, to: &str, confidence: Confidence) -> Link {
        Link { id: id.into(), from: from.into(), to: to.into(), kind: "calls".into(), confidence, unresolved: None }
    }

    fn kind(l: Link, kind: &str) -> Link {
        Link { kind: kind.into(), ..l }
    }

    fn area(id: &str, side: &str, folded: bool) -> Area {
        Area { id: id.into(), side: side.into(), folded }
    }

    fn port(id: &str, kind: &str, side: Side, section: Section, external: Option<&str>) -> Port {
        Port { id: id.into(), kind: kind.into(), side, section, external: external.map(Into::into) }
    }

    fn unresolved(id: &str, from: &str) -> Link {
        Link { id: id.into(), from: from.into(), to: "?".into(), kind: "calls".into(), confidence: Confidence::Guessed, unresolved: Some("dyn".into()) }
    }

    fn pos(item: &str, x: f64, y: f64) -> Position {
        Position { item: item.into(), x, y }
    }

    fn finding(id: &str, rule: &str, link: Option<&str>, level: Level, gates_merge: bool) -> Finding {
        Finding { id: id.into(), rule: rule.into(), link: link.map(Into::into), level, gates_merge, origin: "core".into() }
    }

    /// A handler (driving) calls the domain; a postgres adapter (driven) implements a domain port and talks to
    /// postgres through one port on the driven rail.
    fn hexagon() -> Input {
        let positions = vec![pos("h", 0.0, 0.0), pos("d", 380.0, 0.0), pos("a", 760.0, 0.0)];
        Input {
            rule: Some(ColumnRule::Hexagon),
            has_entry: true,
            columns: ["driving", "domain", "driven", "externals"].map(String::from).to_vec(),
            areas: vec![area("http", "driving", false), area("core", "domain", false), area("pg", "driven", false)],
            items: vec![
                Item { area: Some("http".into()), ..item("h", "svc", "driving", true) },
                Item { area: Some("core".into()), ..item("d", "svc", "domain", false) },
                Item { area: Some("pg".into()), ..item("a", "svc", "driven", false) },
            ],
            links: vec![link("l1", "h", "d", Confidence::Resolved), kind(link("l2", "a", "d", Confidence::Resolved), "implements")],
            externals: vec!["external:sql:postgres".into()],
            ports: vec![port("port:sql:postgres", "sql", Side::Driven, Section::DataStores, Some("external:sql:postgres"))],
            rail_edges: vec![RailEdge { from: "a".into(), external: "external:sql:postgres".into(), port: Some("port:sql:postgres".into()) }],
            positions_without_rails: Some(positions.clone()),
            positions,
            fold: vec![FoldState { visible_items: 3, scale: 1.0, areas_folded: false, label_px: 12.0 }],
            reach: Some(Reach { focus: "d".into(), depth: 1, items: ["h", "d", "a"].map(String::from).to_vec() }),
            ..Default::default()
        }
    }

    fn invariants(v: &[Violation]) -> Vec<&'static str> {
        v.iter().map(|v| v.invariant).collect()
    }

    #[test]
    fn clean_input_passes_everything() {
        let report = check_all(&hexagon());
        assert!(report.is_clean(), "{report}");
    }

    #[test]
    fn empty_input_passes_everything() {
        assert!(check_all(&Input::default()).is_clean());
    }

    #[test]
    fn positions_stable_flags_a_moved_unchanged_item() {
        let mut i = hexagon();
        i.baseline_items = Some(i.items.clone());
        i.baseline_positions = Some(i.positions.clone());
        i.positions[1].y = 1.0;
        let v = positions_stable(&i);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].invariant, "MAP-1");
    }

    #[test]
    fn positions_stable_allows_a_changed_item_to_move() {
        let mut i = hexagon();
        i.baseline_items = Some(i.items.clone());
        i.baseline_positions = Some(i.positions.clone());
        i.items[1].content_hash = "edited".into();
        i.positions[1].y = 1.0;
        assert!(positions_stable(&i).is_empty());
    }

    #[test]
    fn overlays_must_not_move_or_carry_geometry() {
        let mut i = hexagon();
        let mut with = i.positions.clone();
        with[0].x = 5.0;
        i.positions_with_overlays = Some(with);
        i.overlays.push(OverlayRow { kind: OverlayKind::Plan, item: "d".into(), channel: Channel::Fill, has_geometry: true });
        let v = no_state_relayout(&i);
        assert_eq!(invariants(&v), ["MAP-3", "MAP-3"]);
    }

    #[test]
    fn fold_floor_label_and_threshold() {
        let mut i = hexagon();
        i.fold.push(FoldState { visible_items: 200, scale: 0.69, areas_folded: false, label_px: 11.0 });
        let v = fold_floor(&i);
        assert_eq!(v.len(), 3, "{v:?}");
        i.fold[1] = FoldState { visible_items: 200, scale: 0.7, areas_folded: true, label_px: 12.0 };
        assert!(fold_floor(&i).is_empty());
    }

    #[test]
    fn overlay_channels_by_kind() {
        let mut i = hexagon();
        i.overlays = vec![
            OverlayRow { kind: OverlayKind::Sessions, item: "d".into(), channel: Channel::Outline, has_geometry: false },
            OverlayRow { kind: OverlayKind::Findings, item: "d".into(), channel: Channel::Outline, has_geometry: false },
        ];
        let v = overlay_stacking(&i);
        assert_eq!(v.len(), 1);
        assert!(v[0].message.contains("Findings"));
    }

    #[test]
    fn unresolved_links_fold_to_exactly_one_pill() {
        let mut i = hexagon();
        i.links.push(unresolved("u1", "d"));
        i.links.push(Link { unresolved: Some("spawn".into()), ..unresolved("u2", "d") });
        assert_eq!(unresolved_one_pill(&i).len(), 1, "missing pill");
        i.pills.push(Pill { item: "d".into(), kind: "unresolved".into(), count: 2 });
        assert!(unresolved_one_pill(&i).is_empty());
        i.pills.push(Pill { item: "d".into(), kind: "unresolved".into(), count: 1 });
        assert_eq!(unresolved_one_pill(&i).len(), 1, "two pills");
    }

    #[test]
    fn cross_crate_link_needs_public_target() {
        let mut i = hexagon();
        i.items.push(item("x", "other", "domain", false));
        i.links.push(link("l3", "h", "x", Confidence::Resolved));
        let v = cross_unit_on_public_surface(&i);
        assert_eq!(v.len(), 1);
        i.items[3].public = true;
        assert!(cross_unit_on_public_surface(&i).is_empty());
    }

    #[test]
    fn hexagon_grain_points_inward() {
        let mut i = hexagon();
        i.links.push(link("to-domain", "a", "d", Confidence::Resolved));
        assert!(direction_left_to_right(&i).is_empty(), "driven → domain calls are with the grain");
        i.links.push(link("bypass", "h", "a", Confidence::Resolved));
        i.links.push(link("outward", "d", "h", Confidence::Resolved));
        let v = direction_left_to_right(&i);
        assert_eq!(invariants(&v), ["MAP-17", "MAP-17"], "{v:?}");
        i.findings.push(finding("f1", "direction", Some("bypass"), Level::Warn, false));
        i.findings.push(finding("f2", "direction", Some("outward"), Level::Warn, false));
        assert!(direction_left_to_right(&i).is_empty());
    }

    #[test]
    fn only_the_directed_kinds_carry_direction() {
        // Literals, not the constant: a misspelled kind in the constant must fail here (ADR 0033 "Spelling").
        let mut i = hexagon();
        for (n, k) in ["implements", "uses-type", "constructs", "wires", "calls-port", "depends-on-port"].into_iter().enumerate() {
            i.links.push(kind(link(&format!("k{n}"), "d", "a", Confidence::Resolved), k));
        }
        assert!(direction_left_to_right(&i).is_empty());
        let directed = ["calls", "calls-out", "hands-off", "listens-to", "routes"];
        for (n, k) in directed.into_iter().enumerate() {
            i.links.push(kind(link(&format!("c{n}"), "d", "a", Confidence::Resolved), k));
        }
        let flagged: Vec<String> = direction_left_to_right(&i).into_iter().map(|v| v.message).collect();
        for k in directed {
            assert!(flagged.iter().any(|d| d.starts_with(&format!("{k} link "))), "{k} against the grain not flagged: {flagged:?}");
        }
        assert_eq!(flagged.len(), directed.len(), "{flagged:?}");
    }

    #[test]
    fn rail_and_unplaced_endpoints_have_no_direction() {
        let mut i = hexagon();
        i.items.push(item("pg", "svc", "externals", false));
        i.items.push(Item { column: None, ..item("main", "svc", "driving", false) });
        i.links.push(link("to-rail", "d", "pg", Confidence::Resolved));
        i.links.push(link("from-root", "main", "a", Confidence::Resolved));
        i.links.push(link("unresolved", "d", "a", Confidence::Guessed));
        i.links.last_mut().unwrap().unresolved = Some("dyn".into());
        assert!(direction_left_to_right(&i).is_empty());
    }

    #[test]
    fn layers_grain_points_right() {
        let mut i = hexagon();
        i.rule = Some(ColumnRule::Layers);
        i.columns = ["public-api", "internals", "leaves"].map(String::from).to_vec();
        for (item, col) in i.items.iter_mut().zip(["public-api", "internals", "leaves"]) {
            item.column = Some(col.into());
        }
        i.links.push(link("down", "h", "a", Confidence::Resolved));
        assert!(direction_left_to_right(&i).is_empty(), "left → right is with the grain");
        i.links.push(link("back", "a", "h", Confidence::Resolved));
        assert_eq!(invariants(&direction_left_to_right(&i)), ["MAP-17"]);
        i.findings.push(finding("f1", "direction", Some("back"), Level::Warn, false));
        assert!(direction_left_to_right(&i).is_empty());
    }

    #[test]
    fn an_area_cannot_change_column() {
        let mut i = hexagon();
        assert!(columns_are_sides(&i).is_empty());
        i.items[2].column = Some("domain".into());
        assert_eq!(invariants(&columns_are_sides(&i)), ["MAP-2"], "item off its area's side");
        i.items[2].column = Some("driven".into());
        i.areas[2].side = "outbound".into();
        assert_eq!(columns_are_sides(&i).len(), 2, "side not a column, and its item now off-side");
        i.areas[2].side = "driven".into();
        i.items[2].area = Some("nowhere".into());
        assert_eq!(invariants(&columns_are_sides(&i)), ["MAP-2"], "unknown area");
    }

    #[test]
    fn an_area_override_spanning_two_sides_is_rejected() {
        let mut i = hexagon();
        i.items.push(Item { area: Some("pg".into()), ..item("h2", "svc", "driving", false) });
        assert_eq!(invariants(&columns_are_sides(&i)), ["MAP-2"]);
    }

    fn folded() -> Input {
        let mut i = hexagon();
        i.areas.iter_mut().for_each(|a| a.folded = true);
        i.links.push(link("l3", "h", "d", Confidence::Resolved));
        i.links.push(unresolved("u", "h"));
        i.chip_links = vec![
            ChipLink { from: "http".into(), to: "core".into(), count: 2, badged: false },
            ChipLink { from: "pg".into(), to: "core".into(), count: 1, badged: false },
        ];
        i
    }

    #[test]
    fn folded_chip_links_count_their_item_links() {
        let mut i = folded();
        assert!(folded_link_counts(&i).is_empty(), "{:?}", folded_link_counts(&i));
        i.chip_links[0].count = 3;
        assert_eq!(invariants(&folded_link_counts(&i)), ["MAP-7"], "wrong count");
        i.chip_links[0].count = 2;
        i.chip_links.pop();
        assert_eq!(invariants(&folded_link_counts(&i)), ["MAP-7"], "missing chip link");
        i.chip_links.push(ChipLink { from: "core".into(), to: "pg".into(), count: 1, badged: false });
        assert_eq!(folded_link_counts(&i).len(), 2, "missing pg → core, and core → pg folds nothing");
    }

    #[test]
    fn a_finding_on_a_folded_link_badges_the_chip_link() {
        let mut i = folded();
        i.findings.push(finding("f", "domain-must-not", Some("l3"), Level::Warn, false));
        assert_eq!(invariants(&folded_link_counts(&i)), ["MAP-7"], "badge missing");
        i.chip_links[0].badged = true;
        assert!(folded_link_counts(&i).is_empty());
    }

    #[test]
    fn unfolded_areas_draw_no_chip_links() {
        let mut i = folded();
        i.areas[0].folded = false;
        i.chip_links.remove(0);
        assert!(folded_link_counts(&i).is_empty());
    }

    #[test]
    fn reach_is_the_n_hop_neighbourhood_both_ways() {
        let mut i = hexagon();
        i.items.push(item("x", "svc", "domain", false));
        i.links.push(link("l3", "x", "a", Confidence::Resolved));
        i.links.push(unresolved("u", "d"));
        assert!(reach_depth(&i).is_empty(), "depth 1 from d: h (outgoing caller) and a (incoming) both count");
        i.reach.as_mut().unwrap().depth = 2;
        assert_eq!(invariants(&reach_depth(&i)), ["MAP-11"], "x is two hops away");
        i.reach.as_mut().unwrap().items.push("x".into());
        assert!(reach_depth(&i).is_empty());
        i.reach.as_mut().unwrap().depth = 0;
        assert_eq!(reach_depth(&i).len(), 3, "depth 0 is the focus alone");
    }

    #[test]
    fn reach_focus_must_be_an_item() {
        let mut i = hexagon();
        i.reach.as_mut().unwrap().focus = "ghost".into();
        assert_eq!(invariants(&reach_depth(&i)), ["MAP-11"]);
    }

    #[test]
    fn rule_comes_from_the_entry_unless_overridden() {
        let mut i = hexagon();
        assert!(column_rule_from_entry(&i).is_empty());
        i.has_entry = false;
        assert_eq!(invariants(&column_rule_from_entry(&i)), ["MAP-26"], "no entry → layers");
        i.rule_override = Some(ColumnRule::Hexagon);
        assert!(column_rule_from_entry(&i).is_empty(), "the override wins");
        i.rule_override = Some(ColumnRule::Layers);
        i.has_entry = true;
        assert_eq!(invariants(&column_rule_from_entry(&i)), ["MAP-26"]);
    }

    #[test]
    fn externals_are_ports_on_the_rail_in_section_order() {
        let mut i = hexagon();
        assert!(externals_on_rails(&i).is_empty());
        i.externals.push("external:crate:serde".into());
        assert_eq!(invariants(&externals_on_rails(&i)), ["MAP-31"], "external without a port");
        i.ports.push(port("port:crate:serde", "crate", Side::Driven, Section::Libraries, Some("external:crate:serde")));
        assert!(externals_on_rails(&i).is_empty());
        i.ports.push(port("port:http:stripe", "http", Side::Driven, Section::ThirdParty, Some("external:http:stripe")));
        assert_eq!(invariants(&externals_on_rails(&i)), ["MAP-31"], "third-party after libraries");
        i.ports.pop();
        i.ports.push(port("port:x", "declared", Side::Driven, Section::Unresolved, Some("external:crate:serde")));
        assert_eq!(invariants(&externals_on_rails(&i)), ["MAP-31"], "declared is not a MAP-31 kind");
    }

    #[test]
    fn sections_are_ordered_per_side() {
        let mut i = hexagon();
        i.ports.insert(0, port("port:http:POST /orders", "http", Side::Driving, Section::Unresolved, None));
        assert!(externals_on_rails(&i).is_empty(), "driving unresolved before driven data stores is fine");
    }

    #[test]
    fn there_is_no_outbound_column() {
        let mut i = hexagon();
        i.items.push(item("pg-item", "svc", "externals", false));
        assert_eq!(invariants(&externals_on_rails(&i)), ["MAP-31"], "item in the rail column");
        i.items.pop();
        i.items.push(item("external:sql:postgres", "svc", "driven", false));
        assert_eq!(invariants(&externals_on_rails(&i)), ["MAP-31"], "external placed as an item");
    }

    #[test]
    fn docking_moves_nothing() {
        let mut i = hexagon();
        assert!(rail_docking_moves_nothing(&i).is_empty());
        i.positions[2].x = 700.0;
        assert_eq!(invariants(&rail_docking_moves_nothing(&i)), ["MAP-32"]);
        i.positions_without_rails.as_mut().unwrap().pop();
        assert_eq!(rail_docking_moves_nothing(&i).len(), 2, "appears only with rails, and a count mismatch");
    }

    #[test]
    fn ports_have_witness_externals_and_edges_dock_on_them() {
        let mut i = hexagon();
        assert!(ports_from_facts(&i).is_empty());
        i.ports.push(port("port:http:POST /orders", "http", Side::Driving, Section::Unresolved, None));
        assert_eq!(invariants(&ports_from_facts(&i)), ["CANVAS-2"], "no witness external");
        i.ports[1].external = Some("external:http:nowhere".into());
        assert_eq!(invariants(&ports_from_facts(&i)), ["CANVAS-2"], "external not in the facts");
        i.ports.pop();
        i.rail_edges[0].port = None;
        assert_eq!(invariants(&ports_from_facts(&i)), ["CANVAS-2"], "edge ends on no port");
        i.ports.push(port("port:fs:tmp", "fs", Side::Driven, Section::Os, Some("external:sql:postgres")));
        i.rail_edges[0].port = Some("port:missing".into());
        assert_eq!(invariants(&ports_from_facts(&i)), ["CANVAS-2"], "edge ends on an unknown port");
    }

    #[test]
    fn guessed_warns_declared_never_gates() {
        let mut i = hexagon();
        i.links[0].confidence = Confidence::Guessed;
        i.links[1].confidence = Confidence::Declared;
        i.findings = vec![
            finding("g", "domain-must-not", Some("l1"), Level::Block, false),
            finding("d", "domain-must-not", Some("l2"), Level::Warn, true),
            finding("ok", "domain-must-not", Some("l2"), Level::Warn, false),
        ];
        let v = finding_confidence(&i);
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn origin_must_be_core_in_v1() {
        let mut i = hexagon();
        i.findings.push(Finding { origin: "plugin:x".into(), ..finding("p", "r", None, Level::Warn, false) });
        assert_eq!(origin_is_core(&i).len(), 1);
    }

    #[test]
    fn loader_reports_awaiting_data_on_an_empty_dir() {
        let dir = std::env::temp_dir().join("arch-fixtures-empty-golden");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load_golden(&dir), Err(LoadError::AwaitingData)));
    }
}
