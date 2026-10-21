# arch · vocabulary

The terms the spec, the flow pages and the ADRs use, each with the section of `arch-v1-spec.md` that defines it. This file indexes the spec; it does not amend it. Where a page and the spec disagree, the spec wins (spec, preamble).

## C4 mapping (spec §1)

| arch | C4 |
|---|---|
| repo | context |
| unit | container |
| area | component |
| item | code |

## Terms

| term | meaning | source |
|---|---|---|
| **repo** | the repository arch is opened on, with its worktrees; one unit per repo in V1, a board of units in V2, a board of repos in V3 | §1, §3 |
| **unit** | an app, a service, a CLI, or a single library and its closure (private helper crates folded in); in V1 the main `[[bin]]` or the lib, other bins and examples "not analyzed" | §3, §13.7, [ADR 0007](../adr/0007-unit-in-v1.md) |
| **area** | a component of a unit, derived from the module tree, drawn in a column by side; overrides only in `.arch/areas.toml`; no annotation in source | §5, §7, §13.4, [ADR 0004](../adr/0004-areas-derived-no-source-annotation.md) |
| **item** | a piece of code (function, type, trait, module, …) with kind, file, line, visibility and re-export; carries a scope id; everything rust-analyzer calls an item, methods and `impl` blocks included, enum variants and struct fields not; id `<crate>::<module path>::<name>#<kind>` | §3, §7, [ADR 0031](../adr/0031-items-and-item-ids.md) |
| **row** | one line of an open area on the map: a top-level item, with the count of what folds into it (its `impl` blocks and methods; for a module, its `private` loose items); starts closed, opens on demand | §5, [ADR 0037](../adr/0037-open-area-rows.md) |
| **link** | a "uses" relation between items, resolved or unresolved with the reason; of one of 21 kinds in four families (call · type · data · structure) or `refers-to` ([ADR 0033](../adr/0033-link-kinds-and-families.md)); a link that touches a variant or field targets the type and names the member ([ADR 0031](../adr/0031-items-and-item-ids.md)); drawn with the grain of the column rule; a directed link (`calls` · `calls-out` · `hands-off` · `listens-to` · `routes`) against the grain is drawn as a smell ([ADR 0025](../adr/0025-direction-call-family-grain.md)) | §5, §7, `map-invariants.md` |
| **finding** | a rule or lint violation on a link, an item, or an area (`god-module`, [ADR 0035](../adr/0035-what-the-five-lints-check.md)), with a witness and an `origin` (always `core` in V1); blocking or not; a finding on a guessed link warns, never blocks | §6 (Daily), §7, §13.9, [ADR 0009](../adr/0009-confidence-levels.md) |
| **witness** | the source of a shown fact: `file:line`, a contract, a tool output; the host re-reads every locator | §1 (witness rule) |
| **rule** | a dependency rule in `.arch/rules`: subject · must not · targets · level | §7, §13.6 |
| **lint** | a lint setting in `.arch/rules`: `block` · `warn` · `off`; five lints on by `arch init`: `god-module` (an area over 40 % of the unit's lines) · `cycle` (areas that depend on each other) · `leaky-port` (a port's signature naming adapter technology) · `speculative-abstraction` (a trait with one implementor) · `unresolved-dyn` (a port called with no implementor or several, none wired) | §1, §13.6, [ADR 0032](../adr/0032-rules-allows-format-five-lints.md), [ADR 0035](../adr/0035-what-the-five-lints-check.md) |
| **allow** | an allowed site in `.arch/allows`, keyed by site (an area as `area:<name>` for `god-module`), person-only; names its rule by wording, or a lint by name | §6 (Daily), §7, [ADR 0032](../adr/0032-rules-allows-format-five-lints.md), [ADR 0035](../adr/0035-what-the-five-lints-check.md) |
| **plan** | a session that has not run: elements, groups and gates in `plan.toml`; a draft lives in the cache until Accept | §6 (Plan and delegate), §13.20 |
| **element** | one planned change: an intention and a site; id `sha256(session · intention · site)[:8]`, display label `E3`; owned by `plan.toml`, nothing in source | §6, §13.21, [ADR 0021](../adr/0021-element-identity.md) |
| **group** | elements grouped by dependency by the fixed core shaper; one gate per group; drawn as lanes on the session card; produced by policies in V2 | §3, §6, §8 |
| **gate** | what runs when a group ends: commands, `arch check`, the judge; rows on the Checks tab with the output as witness; a fix-round budget of 2 | §6 (Session), §8 |
| **session** | a plan that has been accepted (branch, worktree, thread, records), or a detected `you · <worktree>` session; grouped Planning · Yours · Needs you · Running · In review · Done | §4, §6, §7 |
| **sub-agent** | the agent working on one element of a session; may only write its element's files (the core veto) | §8, §13.12 |
| **scope** | the worktree the shell is about: `main`, a focused session's worktree, or a detected `you` session; shown by the bar's scope selector | §4 |
| **Focus** | making a session the scope (row button, card button, ↩, a chip, the bar's selector); isolates the Sessions view on it; Esc unfocuses | §4 |
| **Lock** | pins a Focus; a plan saved without delegating becomes a locked `you` session | §4, §6 |
| **take over** | pause a session, then take over one element by hand; hand back is written in the composer and lands in the thread; the plan stays the session's | §6 (Session) |
| **remark** | in review: `comment · no change needed` or `ask for a change`; a change asked becomes a plan element with the remark as intention and the hunk's item as site | §6 (Review and merge), §13.22 |
| **record** | a file under `.arch/sessions/<id>/records/`: gate outputs, judge verdicts, denials, overrides, resolution records; archived to a git notes ref at merge | §7, §13.3 |
| **resolution record** | the record of how a conflict was resolved (agent or by hand) | §6, §10, [ADR 0019](../adr/0019-conflicts-resolve-element.md) |
| **What's new** | the one chip and pane for what changed since you looked: map delta · findings ± · unplaced · a session told at idle · tests; also what a running session is told at its next turn boundary | §4, §6 |
| **port** | an external the unit touches, of kind rpc · http · cli · topic · crate · sql · pub · redis · fs · tty · declared, on the unit's flat sides; the public surface (ports) is computed from V1 | §3, §5 |
| **rail** | (shell) the 56 px activity rail Sessions · Files · Findings; (map) the unit's flat sides that hold the ports, in fixed sections: platform services · third-party · events · data stores · os · libraries · unresolved | §4, §5 |
| **node** | a box on the canvas (unit, area, item) rendered at a tier by its on-screen width; never resizes itself | §5 (CANVAS-1) |
| **tier** | the rendering level of a node: mini 110 · chip 380 · card 760 · open 900 px, hysteresis 40, cross-fade on swap | §5 (CANVAS-1) |
| **kind** | the kind of an item (fn, struct, trait, mod, …), of a port (see **port**), or of a plugin (five kinds, V2) | §5, §7, §8 |

## Related

- Confidence of a fact or link: `resolved` · `guessed` · `declared` (§13.9).
- Context pack: the standard input to planner, sub-agents, judge and Ask (§13.5).
- Driver: the agent runtime behind `AgentDriver` (Claude Code first) (§9). Forge: the code host behind the `Forge` trait (GitHub first) (§13.18).
