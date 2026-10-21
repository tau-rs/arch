# arch · roadmap

From `spec/arch-v1-spec.md` §3, kept current by the product chat. A change here is an ADR first.

| | scope | contents |
|---|---|---|
| **V1** | one unit per repo (an app, a service, a CLI, or a single library; private helper crates folded in) | everything in spec §5–§9; hexagon **and** layers; externals at the interface tier (ports for what the repo touches); framework-held entries; whiteboard tools on the unit sheet; the shell; `.arch/` written by CLI or by hand (no onboarding funnel); **no plugin system**: the groups and gates of the plan, the checks, the driver are core and fixed |
| **V1.x** | | GitLab forge adapter ([ADR 0018](adr/0018-forge-github-first.md)); the first plugins as hooks once the plugin spec is folded in (`handoffs/handoff-plugin-system-spec.md`) |
| **V2** | units · plugins | multi-crate repos: unit grouping proposed and kept in `.arch/units`; repo board with free placement and portals; hierarchical fold (unit › crate › module › item, modules as default sub-areas); profiles; trait families; sub-areas by kind; sibling plans; onboarding gains a units step; **the plugin system** (five kinds, marketplace, lockfile, PLG-1…10) with the first policies (waves) and providers; a second provider as judge ([ADR 0013](adr/0013-judge.md)) |
| **V3** | system | multi-repo board; edges from contracts, topics, shared crates, config, or declared and marked; flow overlay across repos; plans spanning repos; runtime traces as witnesses |

## Present in the V1 data model so V2/V3 are additions (spec §3)

- a scope id on every item
- the public surface (ports) computed
- `.arch/board` schema (empty)
- zoom depths named
- commits as facts
- groups and gates in the plan schema (fixed core shaper in V1, policies in V2)
- an `origin` field on every finding, check and overlay row (always `core` in V1)

## Out of V1 (spec §11)

Onboarding funnel and firsts; the plugin system entirely (host, registry, lockfile, Plugins tab, policies, providers, layouts, extra drivers); units, board placement, profiles, trait families, sub-areas by kind, sibling plans (V2); multi-repo, cross-repo edges, flow overlay, runtime traces (V3); parallel groups in the scheduler.

## Open items carried into implementation (spec §12)

- sett: `group` row kind for session-card lanes; the activity rail as a component; layers column tints; the layers direction (public API left) in the map stories; the PoC's fixture data exported.
- Fact model: measured sizes of the four real repos replace the approximations.
- V1 onboarding substitute: `arch init` ([ADR 0006](adr/0006-arch-init.md)).

## Status

| repo | state |
|---|---|
| `arch-design` | seeded 2026-10-02; ADRs 0001–0029 (2026-10-03) |
| `arch` · `arch-app` · `sett` · `arch-fixtures` | see each repo's README for the ADR it is synced to |
