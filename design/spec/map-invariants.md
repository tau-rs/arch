# Map invariants · MAP-1…33, CANVAS-1…2

The contract the map keeps for every flow, as testable statements, one owner and one check each. Answers arch-fixtures FINDINGS.md F-1 and F-9 (tau-rs/arch-design#8, #13). Sources: `flows/map-focus.html` (MAP-1…28), `flows/map-real-projects.html` (MAP-25…29), `handoffs/handoff-visualizer-poc.md` (MAP-30, 31, CANVAS-1, 2), `handoffs/handoff-sett-rebase.md` (MAP-32), [ADR 0037](../adr/0037-open-area-rows.md) (MAP-33), spec §5 for every amendment. Spec §5 wins over a page where they differ.

**Owners.** *fixtures*: checkable from a golden facts/views pair; the check is a function in `arch-fixtures/checks/invariants.rs` (names in the table; existing ones are marked). *app* / *sett*: a story test in `arch-app` or `sett`, run against `fixtures-for-ui/`; the table says what the story asserts. *V2*: holds vacuously in V1 (one unit, no profiles); the check exists where it is cheap and bites in V2. *dropped* / *superseded*: kept in the numbering, never checked.

## MAP

| # | testable statement | V1 amendment (spec §5) | owner | check |
|---|---|---|---|---|
| MAP-1 | Positions inside a unit come from facts only; no stored coordinates; unchanged items are byte-identical between saves. | as is | fixtures | `positions_stable` (exists) |
| MAP-2 | Columns are sides, computed; an area cannot change column: every item's column equals its area's side under the unit's rule. | as is | fixtures | `columns_are_sides` |
| MAP-3 | No state ever re-layouts: selection, focus, overlays, sessions, plans, findings paint only; positions with every overlay on equal positions with none. | as is | fixtures | `no_state_relayout` (exists) |
| MAP-4 | Fold, never shrink: one label size across every fold state; above ~80 visible items areas fold to chips with counts; the fold keeps the footprint with a floor at 0.7 of the opening scale. | amended: one label size, ~80 threshold, 0.7 floor | fixtures (footprint, threshold, floor); sett (label size ≥ token) | `fold_floor` (exists); story: the chip and card labels at the type token |
| MAP-5 | Levels: repo › unit › areas › items, remembered per view and per branch; repo › unit is not a level change in V1; code opens as a tab only by double-click or ↩. | reworded | app | story: ↩ and double-click open a tab, single click does not; level remembered across a branch switch |
| MAP-6 | ~~One expanded area at a time by default; ⌘-click expands another.~~ | **dropped** | dropped | none |
| MAP-7 | Links between folded chips carry counts equal to the item links they fold; a finding on a folded link badges the chip. | as is | fixtures | `folded_link_counts` |
| MAP-8 | Sub-areas fold recursively and are declared like areas. | V2 (modules as default sub-areas, MAP-27) | V2 | none in V1 |
| MAP-9 | The selection is one, shared by map, files, preview, Ask and Review. | as is | app | story: select on the map, the same id is current in Files, preview, Ask, Review |
| MAP-10 | ⌘P finds items, files, areas and rules; ↩ selects and opens at the right level. | as is | app | story: palette over the smallsvc fixture, one hit per kind, ↩ lands on it |
| MAP-11 | Focus fades everything outside the Reach ring; the ring's depth comes from the Reach stepper. | as is | fixtures (the Reach set at depth n); app (the fade) | `reach_depth`; story: items outside the set are faded |
| MAP-12 | Arrows walk links; `f` fits; ⌘1 is the Map tab. | reworded: `f` fit (spec §5); ⇧1/⇧2 pending tau-rs/arch-design#3 | app | story: arrow from an item lands on a linked item; `f` fits the level |
| MAP-13 | The minimap appears only when the map exceeds the viewport. | as is | app | story: small fixture → no minimap; zed fixture → minimap |
| MAP-14 | Four overlays, sessions · plan · findings · delta, each a toggle; stacking: outline = session, fill = plan or finding; no overlay groups items spatially; an item may carry all four. | as is | fixtures (rows, stacking, no geometry); sett (paint) | `overlay_stacking` (exists); story: one item with all four overlays |
| MAP-15 | No animation on the map except one pulse on a detected change; `prefers-reduced-motion` removes the pulse. | as is | app / sett | story: a change event pulses once (the 700 ms ring); reduced-motion → no ring |
| MAP-16 | Unresolved paths (dyn, spawn) are drawn dashed amber and never merged into resolved links; folded to one pill per item; Reach expands them. | as is (with 28: the pill) | fixtures (pill, never merged); sett (dashed amber) | `unresolved_one_pill` (exists); story: the unresolved pill token |
| MAP-17 | A directed link (`calls` · `calls-out` · `hands-off` · `listens-to` · `routes`) against the grain of the unit's column rule is drawn dashed as a smell; no other link kind carries a direction. | amended by [ADR 0025](../adr/0025-direction-call-family-grain.md), kinds named by [ADR 0033](../adr/0033-link-kinds-and-families.md) | fixtures | `direction_left_to_right`, narrowed per ADR 0025 (exists) |
| MAP-18 | Editing tools, toolbar · selection box with pill · tray, are identical everywhere areas are editable. | as is | app | story: the same toolbar on the unit sheet and in a focused area |
| MAP-19 | Drawing an area is constrained to one column. | as is | app (the gesture); fixtures (an override spanning two sides is rejected, via MAP-2) | story: the drag clamps at the column edge; `columns_are_sides` |
| MAP-20 | Dropping on a folded chip places into that area. | as is | app | story: drop on a chip, `areas.toml` override names that area |
| MAP-21 | Every edit is undoable until Keep; Keep is one change in Changes. | as is | app | story: n edits, n undos, Keep → one row in Changes |
| MAP-22 | The repo screen overlays every session at once. | as is | app | story: two sessions in the fixture, both outlines on the unit sheet |
| MAP-23 | ~~WebGL for items and links at scale; DOM for the chrome.~~ | **superseded**: rendering is DOM (spec §5, sett ADR 0001) | superseded | none |
| MAP-24 | A recompute touches only changed items; unchanged positions are byte-identical between saves. | as is | fixtures | `no_state_relayout` / `positions_stable` (exist) |
| MAP-25 | Units sit above areas: a crate or crate group with its own layout; the repo level draws units. | V2 (one unit in V1; board schema exists) | V2 | none in V1 |
| MAP-26 | Two column rules, hexagon (unit has an entry) and layers (no entry), chosen per unit, overridable in `.arch/areas.toml`; each defines its grain (ADR 0025). | direction fixed → ADR 0025 | fixtures | `column_rule_from_entry` |
| MAP-27 | Folding is hierarchical: unit › crate › module › item; modules are the default sub-areas. | V2 | V2 | none in V1 |
| MAP-28 | One profile at a time; other-profile items ghosted with their feature. (The "unresolved folded to a pill" half is MAP-16 in V1.) | V2 (profiles) | V2 | `unresolved_one_pill` covers the V1 half |
| MAP-29 | A trait whose impls exceed a threshold becomes a family: one item with the count and a per-crate breakdown; its impls draw no links here. | V2 | V2 | none in V1 |
| MAP-30 | Cross-unit links land on public surfaces; a link into a non-pub item of another unit is a finding. | V2 (vacuous with one unit) | V2 | `cross_unit_on_public_surface` (exists; vacuous in V1) |
| MAP-31 | Externals live at the interface tier only: every external is a port of kind rpc · http · cli · topic · crate · sql · pub · redis · fs · tty · declared on the unit's flat sides, in the fixed section order platform services · third-party · events · data stores · os · libraries · unresolved; there is no outbound column. | as is | fixtures | `externals_on_rails` |
| MAP-32 | Edges dock to ports and detour rather than move anything; a neighbour's matching port is wired straight; the rail ends with an unresolved section. | as is | fixtures (docking moves no position); sett (the rail with its unresolved section) | `rail_docking_moves_nothing`; story: `sett-rail` with the unresolved section |
| MAP-33 | An open area draws one row per top-level item: an `impl` block folds into its type's row when the type is in the same area (`Item.self_type`), an associated item with its `parent`, a `private` item with no `parent` (not a `mod` or `impl`) into its module's row; every row starts closed; a link, finding, overlay or Reach hit on a folded item lands on its row with a count; folded items are not visible items. | new: [ADR 0037](../adr/0037-open-area-rows.md) | fixtures (rows and counts from facts); sett (the folded count); app (open and close) | `rows_fold_into_owner`; story: a row's folded count, opened and closed |

## CANVAS

| # | testable statement | V1 amendment (spec §5) | owner | check |
|---|---|---|---|---|
| CANVAS-1 | Node tiers by on-screen width with the same thresholds everywhere, mini 110 · chip 380 · card 760 · open 900 px, hysteresis 40; a swap cross-fades; the camera moves; the node never resizes itself. | thresholds fixed in spec §5 (the PoC's 160/420 are superseded) | app / sett | story: zoom continuously, record tier swaps at the four thresholds ± 40, assert the node box is constant |
| CANVAS-2 | Ports come from facts (every port has a witness external); an edge to an external ends on that external's port. V2: sockets on the board. | V1: ports on the rails (MAP-31/32) | fixtures | `ports_from_facts` |

## Checks to add in `arch-fixtures/checks/invariants.rs` (arch-fixtures #5)

`columns_are_sides` · `folded_link_counts` · `reach_depth` · `column_rule_from_entry` · `externals_on_rails` · `rail_docking_moves_nothing` · `ports_from_facts` · `rows_fold_into_owner`; and `direction_left_to_right` narrowed to the five directed kinds with the grain read from `rule` (ADR 0025, ADR 0033). Existing and unchanged: `positions_stable` · `no_state_relayout` · `fold_floor` · `overlay_stacking` · `unresolved_one_pill` · `cross_unit_on_public_surface`; `finding_confidence` (ADR 0009) and `origin_is_core` (spec §3) are not MAP numbers and stay.

## Story tests to add (arch-app / sett, against `fixtures-for-ui/`)

MAP-4 (label token) · 5 · 9 · 10 · 11 (fade) · 12 · 13 · 14 (paint) · 15 · 16 (dashed amber) · 18 · 19 · 20 · 21 · 22 · 32 (rail) · 33 (folded count, open and close) · CANVAS-1. These are the invariants FINDINGS F-9 lists as untestable from facts; each has exactly one check, there.
