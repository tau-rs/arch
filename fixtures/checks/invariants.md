# Invariants → checks

Where each map invariant is checked. The statements, owners and V1 amendments live in
[`tau-rs/arch-design` `spec/map-invariants.md`](https://github.com/tau-rs/arch-design/blob/main/spec/map-invariants.md);
this file does not copy them. Each check's doc comment in [`invariants.rs`](invariants.rs) holds the statement it tests.

Every number appears once. A *fixtures* number points to exactly one function; *app* / *sett* numbers are story
tests in `arch-app` or `sett`; *V2* holds vacuously in V1; *dropped* / *superseded* are never checked.

| # | owner | checked by |
|---|---|---|
| MAP-1 | fixtures | `positions_stable` |
| MAP-2 | fixtures | `columns_are_sides` |
| MAP-3 | fixtures | `no_state_relayout` |
| MAP-4 | fixtures + sett | `fold_floor`; label size at the type token: story in sett |
| MAP-5 | app | story in arch-app |
| MAP-6 | dropped | none |
| MAP-7 | fixtures | `folded_link_counts`; reading open in tau-rs/arch-design#120 |
| MAP-8 | V2 | none in V1 |
| MAP-9 | app | story in arch-app |
| MAP-10 | app | story in arch-app |
| MAP-11 | fixtures + app | `reach_depth`; reading open in tau-rs/arch-design#119; the fade: story in arch-app |
| MAP-12 | app | story in arch-app |
| MAP-13 | app | story in arch-app |
| MAP-14 | fixtures + sett | `overlay_stacking`; paint: story in sett |
| MAP-15 | app / sett | story in arch-app / sett |
| MAP-16 | fixtures + sett | `unresolved_one_pill`; dashed amber: story in sett |
| MAP-17 | fixtures | `direction_left_to_right`, narrowed per ADR 0025 |
| MAP-18 | app | story in arch-app |
| MAP-19 | app + fixtures | the gesture: story in arch-app; an override spanning two sides: `columns_are_sides` (via MAP-2) |
| MAP-20 | app | story in arch-app |
| MAP-21 | app | story in arch-app |
| MAP-22 | app | story in arch-app |
| MAP-23 | superseded | none (rendering is DOM) |
| MAP-24 | fixtures | `positions_stable` (unchanged positions byte-identical between saves) |
| MAP-25 | V2 | none in V1 |
| MAP-26 | fixtures | `column_rule_from_entry` |
| MAP-27 | V2 | none in V1 |
| MAP-28 | V2 | the V1 half (unresolved folded to a pill) is MAP-16: `unresolved_one_pill` |
| MAP-29 | V2 | none in V1 |
| MAP-30 | V2 | `cross_unit_on_public_surface` (vacuous with one unit) |
| MAP-31 | fixtures | `externals_on_rails`; facts disagree, tau-rs/arch-design#118 |
| MAP-32 | fixtures + sett | `rail_docking_moves_nothing`; the rail with its unresolved section: story in sett |
| CANVAS-1 | app / sett | story in arch-app / sett |
| CANVAS-2 | fixtures | `ports_from_facts`; facts disagree, tau-rs/arch-design#118 |

Checks that are not map invariants: `finding_confidence` (ADR 0009) and `origin_is_core` (spec §3).

Budgets are in [`budgets.toml`](budgets.toml) (ADR 0026), one benchmark per budget.
