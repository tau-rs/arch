# arch · handoff · plugin system (custom visuals, engine tweaks, implementation policies)

Purpose of this side-work: design the plugin system of arch, that is the extension points and their contracts, so that (a) visuals can be added without touching the core (new overlays, panes, map tiers, inlays), (b) the engine can be tweaked (fact providers, rules and lints, column rules, agent drivers, edge derivations), and (c) a team can **enforce a way of implementing things** (ordering, parallelism, gates, commit shape, required checks). The deliverable is the plugin system, not any particular plugin: the policies and visuals mentioned below are **test cases for the contract**, used to check that the shape is right, and are not to be designed in depth here. Output comes back to the product chat for verification; the result is folded into the V1 spec as the extension contract, then handed to Claude Code.

## 1. Context you need (from the product chat)

arch is a desktop dev tool for Rust codebases built as an Eclipse Theia product. It computes an architecture map from rust-analyzer facts and lets a person and coding agents work on a repo through that map. Everything is read from files and witnessed; nothing happens without a fact behind it.

Vocabulary: **repo · unit · area · item · link · finding · witness · rule · lint · plan · element · session · sub-agent · remark · resolution record · What's new**.

Principles that constrain plugins:
- **P-1** every action has a manual path and an agent path, both visible; manual is the default door. A plugin cannot add an agent-only action.
- **P-2** arch watches the repo and its worktrees and rebuilds from files; it is a reader, never an owner. A plugin cannot hold state that isn't derivable from files plus `.arch/`.
- **Witness rule**: every fact shown has a source (file:line, a contract file, a tool output). A plugin that contributes facts contributes witnesses.
- **No modals, no banners**: state is a chip in the Actions strip, a pill where the subject is named, the frame's colour, or a What's new line.
- **The map never re-layouts for state**; plugins paint, they don't move.

Reference artifacts (read in this order):
1. base flow (shell, map, selection, Reach): https://claude.ai/artifact/RPXYVep3QJFZuZ68zwSkFY
2. plan flow (focus, intention, planner thread, Accept · delegate | Save plan): https://claude.ai/artifact/TvLPb23m3CxwhLZooqdMxi
3. session flow (repo screen, session card, live editing, questions, deviations, session bar, step in): https://claude.ai/artifact/Tv9ST2JoKzRxDu5dLDCh7n
4. review flow (hunks, remarks → plan delta, checklist, verdict): https://claude.ai/artifact/UrRzr9VVzXcb3MP87sTWyT
5. merge flow (gated button, pipeline from the forge, result): https://claude.ai/artifact/2oskEYshxjgwYkyH1kHmxj
6. sync flow (change detected, What's new, reconcile with impact, stale-write guard, told at idle): https://claude.ai/artifact/CGrQsXSw9ghaN8GmQkQMEJ
7. findings flow (finding chip, fix card, allow, rules): https://claude.ai/artifact/Qcst7B3a2PWsPzWbZSEggN
8. map focus (anatomy, folding, overlays, invariants MAP-1…31): https://claude.ai/artifact/PTACFVEdRRHB4uYKW7dgTr
9. V2 canvas (zoom levels, tiers, sockets, flow overlay): https://claude.ai/artifact/K8ur1qKoCYb3up5c1T8Cwa

Technical chain: `arch-rust` (ra_ap_* facts) → `arch-facts` (store) → `arch-views` (layouts, folding) → `arch-server` (query API, MCP, drivers) → clients (Theia UI, Neovim, CI). Agents are provider-agnostic behind an `AgentDriver` trait (Claude Code first). Every agent write goes through arch's tool layer (stale-write guard, content hash from last read). The plan is `.arch/plan` with its thread; areas are one `//! @arch area:` line per file plus `.arch/areas`; rules are `.arch/rules`.

## 2. The question to answer

What is the minimal set of extension points, and the contract for each, such that:
- the core stays "exact and small" (nothing in the core that a plugin could do);
- a plugin can add a visual, a fact, a rule, a layout, a driver, or an implementation policy;
- plugins are declared per repo in `.arch/plugins` so a team's policies travel with the code, and agents see them through MCP exactly as the UI does;
- the policies and visuals listed in section 4 can each be written as a plugin with no core change.

## 3. Candidate extension points (start here, prune or merge)

| point | what it contributes | consumed by | V1 core or plugin? |
|---|---|---|---|
| **fact provider** | items, links, witnesses from a source other than rust-analyzer (OpenAPI, proto, Helm values, SQL migrations, OTel traces) | facts store, map, Ask | core: rust-analyzer, cargo; plugin: everything else |
| **side / layout rule** | how a unit's items are assigned to columns (hexagon, layers, a team's own) | arch-views | core: hexagon, layers; plugin: others |
| **rule and lint** | a check over facts producing findings with witnesses and a level (warns / blocks), with a fix card template | findings flow, merge checklist, CI `arch check` | core: dependency rules + 5 lints; plugin: team lints |
| **overlay** | a paint pass over the map (colour, dash, badge, pill) keyed by item or link; never positions | map tab bar toggles | core: sessions, plan, findings, delta; plugin: e.g. coverage, ownership, hot paths |
| **pane** | a right-pane context or a left-pane view, with the fixed verb bar + composer pattern if it talks | shell | core: Code/Reach, framer, planner, session, fixer, review; plugin: others |
| **inlay** | an editor annotation at a witness line | file tab | core: witness, planned, you, session, finding; plugin: others |
| **tier content** (V2) | what a node shows at tiers A/B/C, sockets for a kind of external | canvas | core: Rust pub surface, routes, rpcs, topics; plugin: other contracts |
| **edge derivation** (V3) | how two repos are linked (contract matching) with witnesses | system level | plugin |
| **agent driver** | a provider behind `AgentDriver` | sessions | core: Claude Code; plugin: Codex, tau, Hermes |
| **planner template** | how a plan is drafted (from scratch, from a sibling, from a template, from an ADR) | plan focus | core: scratch, sibling; plugin: others |
| **implementation policy** | constraints on how a plan is realized by a session: ordering, parallelism, gates, commit shape, required checks | plan focus (Accept), session (sub-agents, bar), review (checklist lines), merge (gate) | plugin |
| **query** (Ask catalogue) | a named computed query the model may run, with its witness format | Ask | core: entry, path, why, check, lint, metrics; plugin: others |
| **CI action** | what `arch check` runs in the pipeline and how its output is read back into the merge pane | merge | core: check; plugin: others |

Decide for each: contract (inputs, outputs, witnesses), where it appears in the UI (the existing surfaces only: chip, pill, overlay toggle, pane, checklist line, fix card, inlay, What's new line), how it's declared, how it's versioned, and how an agent sees it through MCP.

## 4. Test cases for the contract (do not design them; use them to break the design)

Each candidate extension point must be able to carry these without a core change. Write each as a one-paragraph sketch against the contract, enough to find what the contract lacks, no more.

- **An implementation policy with parallelism and gates** (the person's earlier harness work: elements grouped into waves by dependency, N sub-agents per wave in parallel, an external verification gate between waves, a binary judge after each wave, a fix-round budget then escalate). What it needs from the contract: to read the plan at Accept and annotate it, to constrain how the session spawns sub-agents, to add checklist lines to review and merge, to paint on the map (a band per wave) and the session card (lanes), to put a stop hook in arch's tool layer, and to stay visible when no session runs (P-1: gates become chips on the manual path).
- **A TDD-first policy**: a test element must be realized before the element it covers; the gate is "red then green"; the finding when an agent writes production code first.
- **A commit-shape policy**: one commit per element, message template, no WIP commits on the branch; a finding in review otherwise.
- **An ownership overlay** from CODEOWNERS: paints items by owning team; a review checklist line "owners of touched areas notified".
- **A fact provider** from Helm values or OpenAPI: new items (config keys, routes) with witnesses, linked to the code that reads or serves them; appears in the map and in Ask's queries.
- **A layout rule** a team invents (say, columns by bounded context rather than by side): must plug where hexagon and layers plug.
- **A second agent driver** (Codex or tau) behind `AgentDriver`, including how its tool calls pass through the stale-write guard.

If a case needs a UI surface the shell doesn't have, that is a finding against the shell; bring it back to the product chat rather than inventing a surface in the plugin.

## 5. Architecture questions to answer

1. **Where plugins run**: Theia extensions for pure UI (panes, overlays, inlays); arch-server plugins for facts, rules, layouts, drivers, policies. One manifest for both, loaded from `.arch/plugins` (repo-scoped) and the user's settings (user-scoped), repo wins.
2. **Isolation**: engine plugins as WASM components (wasmtime) with a capability list (read facts, emit findings, call a driver), or as subprocesses speaking the same JSON-RPC as drivers; UI plugins as ordinary Theia extensions reading the query API. Decide and justify; the stale-write guard and the witness rule must be unbypassable by a plugin.
3. **Contracts as data**: a plugin's manifest declares what it contributes and which UI surfaces it uses; the shell has no plugin-specific code; an unknown plugin degrades to its chips and checklist lines.
4. **Agents and plugins**: everything a plugin contributes is visible through MCP (a rule, a query, a policy's gates), so an agent obeys the same policy as the UI shows. A policy that only the UI knows is a bug.
5. **Versioning and trust**: plugins pinned in `.arch/plugins` by name and version (and hash); repo-scoped plugins run only after the person accepted them once (a chip: "2 plugins declared by this repo · review · enable").
6. **What is NOT pluggable**: the map's positions, the frame, the Actions strip ordering, the witness rule, the stale-write guard, the manual door. Say so.

## 6. Deliverables

1. The extension-point list, pruned, each with: contract (schema or trait), surfaces used, declaration, MCP exposure, V1/V2/V3.
2. The plugin manifest format and the loading/trust model, with the UI touchpoints (the one chip, the settings view) mocked on the shell.
3. For each test case in section 4, a one-paragraph sketch against the contract and the list of what the contract lacked (or a note that it fit). No plugin is designed in depth.
4. The **core hooks list**: what the V1 core must expose for this to work (hooks in the tool layer, the query API surface, the registration of panes, overlays, inlays, rules, layouts, policies), so V1 implementation leaves the sockets in place even if no plugin ships with V1.
5. The **not-pluggable list**, written as invariants with a sentence of justification each.

## 7. Constraints
- Lean: if a point can wait for a second real plugin, mark it V2 rather than designing it now; the test cases exist to validate the shape, not to be built.
- Every mock reuses the existing shell and components (no new surfaces); if a plugin needs a surface that doesn't exist, that's a finding against the shell, bring it back.
- Rust for anything in arch-server; the Theia side is TypeScript; agree the JSON contract between them first.
