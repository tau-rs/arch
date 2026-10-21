# arch · handoff · visualizer proof of concept

Purpose of this side-work: prove that the map can be implemented as designed, from the system level down to code, on a real repository, with the invariants holding at scale. This is a technical spike, not a product build. Output comes back to the product chat for verification before V1 implementation.

## 1. The question to answer

Can we render one canvas with **semantic zoom** (system › repo › unit › code), **computed layouts inside a unit** (hexagon and layers), **free placement above it**, **folding instead of shrinking**, **overlays that never re-layout**, and **zoom tiers** (chip › interface › open), on a real Rust workspace of 200+ crates and tens of thousands of items, within the performance budgets below?

If not, which invariants break, and what would it cost to keep them.

## 2. What the map is (read these)

- Map focus (anatomy, folding, navigation, overlays, editing, invariants MAP-1…MAP-28): https://claude.ai/artifact/PTACFVEdRRHB4uYKW7dgTr
- The map on real projects (Zed, tokio, Bevy, Vector: as designed / units only / fix): https://claude.ai/artifact/PQoTSQ8JU6avRADKDRZjwq
- V2 canvas (four zoom levels, zoom tiers, sockets and wires, flow overlay): https://claude.ai/artifact/K8ur1qKoCYb3up5c1T8Cwa
- Base flow for the shell around the map: https://claude.ai/artifact/RPXYVep3QJFZuZ68zwSkFY

## 3. The model to feed the visualizer

Compute these from rust-analyzer (via its LSP or `ra_ap_*` crates) and `cargo metadata`; write them to a JSON fixture so the visualizer can be tested without the analyzer running:

- **crates** (workspace members), **targets** (bin / lib / example), **crate dependency graph** with counts of depending items.
- **items**: id, kind (fn, struct, enum, trait, impl, mod, macro), file, line, crate, module path, `pub` visibility, re-exported (pub use) or not.
- **links**: item → item (calls, uses, impls, type refs), resolved or **unresolved** (dyn dispatch, spawn boundaries, generic bounds) with the reason.
- **entries**: bin mains, `#[tokio::main]`, framework-held entries (a small table: axum, actix, tonic, bevy App, lambda).
- **externals**: dependency crates with the subset of their public API this workspace touches (the sockets).
- **public surface** per crate: pub items reachable from lib.rs, plus registered routes / rpcs / topics where detectable (axum Router, tonic service impls, string-literal topics).
- **units** (proposal): every bin crate is a seed; a crate depended on by exactly one seed closure joins it; shared or seedless crates are library units; same-folder always-together crates merge.
- **sides** for a unit with an entry: driving (reachable from the entry before a port), domain (no outgoing dependency outside the unit except std), driven (implements a port or wraps an external); **layers** for a unit without an entry: topological depth.
- **areas**: from `//! @arch area:` lines if present, else modules.

Fixture repos: **Zed** (≈210 crates), **tokio**, **Bevy**, **Vector**, plus one small service (≈130 items). Measure real numbers and replace the approximations in the real-projects page.

## 4. The invariants to prove (each is a check)

| id | invariant | how to test |
|---|---|---|
| MAP-1 | positions computed from facts inside a unit; placed and stored above | rebuild twice → byte-identical positions; move a unit node → persists |
| MAP-3 | no state re-layouts | toggle every overlay and selection → zero position deltas |
| MAP-4 | fold, never shrink; every visible label readable | at every level, min label size ≥ token; item count above threshold triggers fold |
| MAP-5/27 | levels repo · units · areas · items; hierarchical fold unit › crate › module › item | open Zed, fold/expand each level; measure |
| MAP-14 | four overlays; outline = session, fill = plan/finding | render a fixture with all four on one item |
| MAP-16/28 | unresolved kept separate; folded to one pill per item | tokio runtime: 38 spawn paths → 1 pill |
| MAP-24 | recompute touches only changed items | edit one file in the fixture; diff positions |
| MAP-25/26 | unit = scope; hexagon with entry, layers without | Zed: 9 units, gpui in layers, zed·app in hexagon |
| MAP-29 | trait families above a threshold | Bevy: Component → one family item, 3,100 impls, per-crate breakdown |
| MAP-30 | cross-unit links land on public surfaces; bypass = finding | count links into non-pub items across units in Zed |
| MAP-31 | externals at interface tier only | axum sockets for a glue service |
| CANVAS-1 | tiers by on-screen size, same thresholds everywhere | zoom continuously; record tier transitions at 160 / 420 px |
| CANVAS-2 | sockets from facts; edges attach to sockets | payments-style fixture: rpc sockets wired to handlers |

Performance budgets (release build, mid laptop): first paint of Zed at the units level < 2 s; fold/expand an area < 100 ms; recompute after one file change < 500 ms; 60 fps pan/zoom at 5,000 visible items; memory for the Zed fixture < 1 GB in the visualizer process.

## 5. Suggested approach (not mandatory)

- Rendering: WebGL for items and links (regl / pixi / custom), DOM for labels above a size, tab bar, pills, tray, minimap. Confirm whether labels can stay WebGL-rendered (SDF text) at the density needed; the mock labels are 9.5–10 px equivalent.
- Layout: column assignment is trivial once sides/layers are known; within a column, order by dependency depth then name; row height fixed; area boxes sized by content; no force-directed anything.
- Folding: precompute per-level layouts; a fold is a switch, not a re-layout.
- Zoom: continuous camera; tier chosen per node from its projected size; snapping to a level when a node exceeds the viewport, with a crossfade; level 1 and 2 are the same code path with stored positions.
- Data: keep the analyzer facts in a local store (sqlite or sled); the visualizer reads a query API, so the same PoC can later run against arch-server.

## 6. Deliverables

1. A runnable PoC (Rust + web, or web-only against the JSON fixtures) that opens the Zed fixture and lets you: zoom from repo (units) to a unit (hexagon or layers) to an area to an item, fold and expand, select and focus, toggle overlays from fixture data, drag a unit node and reload with the position kept, see tier B sockets on one external.
2. The fixture generator (analyzer → JSON) with the five repos' outputs and their measured sizes.
3. A results table: each invariant above with pass / fail / partial and the measured budget numbers.
4. A written list of what did not hold and the proposed change (to the invariant or to the design), for the product chat to decide.

## 7. Out of scope for the PoC

Editing tools (toolbar, tray, area drawing), sessions and plans (use static overlay fixtures), the flow overlay across repos, runtime traces, the Theia integration, dark mode. The PoC proves rendering, layout, folding and zoom; everything else is V1 implementation.
