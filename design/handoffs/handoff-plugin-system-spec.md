# arch · handoff · plugin system: brainstorm, spec, wireflows

Oct 2026. Purpose: take the plugin system from "decided at the contract level" to a full specification with wireflows, so it can be implemented in V1 as hooks and in V1.x as the first plugins. The result comes back to the product chat for verification and is folded into the V1 spec (§8).

## 1. Where things stand (read first)

Decided in the side-chat, do not reopen unless a wireflow breaks it:

- **Five contribution kinds**: facts (provider), query (presented as check · overlay · inlay · Ask entry), layout, policy, driver. Thirteen candidates were pruned to these; overlay/inlay/rule/Ask-query are one kind with a presentation; CI action is cut (`arch check` runs checks; external reports enter as facts); tier content and edge derivation are fact providers with a scope.
- **Runtime**: facts, query, layout and policy are WASM components (wasmtime, world `arch:plugin@1`), grant list, no write import; drivers are JSON-RPC subprocesses (hello · start · send · stop) and the core attributes every worktree write.
- **Distribution**: marketplace model, installed per person from registries (public default + private; publisher verification, signed artifacts, install counts, no star ratings); `.arch/plugins` is a lockfile with `[[require]]` and `[[recommend]]` pinned by version and hash, optional `path =` for sideloaded; installing a required pin is the acceptance for that repo on that machine; a required plugin not installed shows `not-evaluated` rows (blocking checks keep merge gated, delegate waits, the manual path stays open, CI installs the lockfile headless). Installing is a trust action: an agent may propose, the click is the person's.
- **UI**: no permanent chrome; one Plugins center tab (search, six kind chips, All · This repo · Installed, registries; one sectioned list), detail in the inspector, README as a tab; doors: `+` where a kind is chosen, the required chip, origin tags on contributed lines, Ask ⌘K, the status-bar count; management is ⋯ per installed row; requiring for a repo is a hunk in `.arch/plugins` that leaves through git.
- **Not pluggable (PLG-1…10)**: positions, surfaces, the frame and the chips' order, the witness rule (host re-reads every locator), the stale-write guard (runs first, plugins have no write import), the manual door, reader-not-owner, trust outside the repo, one view for person and agent, reserved paint colours.
- **Core hooks V1 leaves in place**: plugin host and grants; registry client and install store; acceptance store; provider registry with witness validator and kind registry; query registry with checks; layout registry; plan schema with namespaced element attributes and shaping; scheduler honouring groups and a gate runner (check · command with expect · judge); tool-layer pipeline guard → policy veto with call context; event bus; driver JSON-RPC with confinement and write attribution; host-mediated forge read; row renderer per surface with `not-evaluated`; MCP exposure of every registry.

Artifacts: plugins contracts https://claude.ai/artifact/JGtK8hmBmkakcvRkwqWkmy · plugins wireflow https://claude.ai/artifact/CVJt77dw8uCTQgYvkmsNYF · the shell https://claude.ai/artifact/KENuEgpvSjX4p7jRTGxebn · session on the shell (gate, gate failed, deviation as denied write) https://claude.ai/artifact/Eu2VyYvcjsDziTCoB271Dc · plan on the shell (groups and gates from a policy) https://claude.ai/artifact/5yqj554jto8B96bvepuAra · review and merge (checklist lines, not-evaluated) https://claude.ai/artifact/1MxQnkAfsxh61hAQWkvkqU · V1 spec `arch-v1-spec.md` (§8, §9, §12).

Principles that bind the work: P-1 agent door first, manual always; P-2 reader not owner; the witness rule; nothing moves on the map for state; no modals; everything a plugin contributes is visible through MCP.

## 2. What is not yet specified (the work)

The contracts say *what* a plugin can contribute. Missing: *how it is written, tested, published, found, trusted, used, observed, updated and removed*, on every surface, by a person and by an agent, and how the core hooks behave in edge cases. Produce the spec and the wireflows for each of the following.

### 2.1 Authoring
- The SDK: a Rust crate (`arch-plugin`) with the WIT world, derive macros for each kind, a test harness that runs a plugin against a fixture repo (the PoC's ripgrep / zero2prod fixtures) and asserts rows, witnesses, vetoes.
- Scaffolding: `arch plugin new <kind>`; what the generated plugin does on day one for each kind.
- Local loop: sideload with `path =`, hot reload on rebuild, the `sideloaded` label on every contributed row, the Checks tab showing the plugin's own test run.
- Witness validation at author time: a row with a locator the host cannot re-read is a build error, not a runtime surprise.
- Versioning: semver, what counts as breaking for each kind (a check id disappearing, a layout changing columns, a policy changing a gate), and what the lockfile does on a breaking bump.

### 2.2 Publishing
- Registry format: static index over git tags (name, versions, hashes, signature, manifest, README); private registries by URL with auth; mirroring.
- Publisher verification and signing; what the UI shows for unverified publishers.
- Docs generated from the manifest (contributions in arch's words, needs, grants).

### 2.3 Finding and installing (person)
- The Plugins tab in detail: search, kind chips, filters, the sectioned list (required · recommended · installed · marketplace), the inspector detail (what it contributes, where it will appear, what it needs, grants, versions, publisher), README as a tab.
- The `+` doors: end of the overlay row, driver picker, layout picker, rules section, Ask ⌘K ("is there a check for…"). Each a wireflow: from the door to the installed, acceptance done, row visible, with no detour through settings.
- Trust: the acceptance moment (grants listed in plain words, what the plugin can read, what it can veto), re-acceptance on version, hash or grant change, the repo-required chip, `not-evaluated` rows before acceptance.
- Updates: per person, per registry; what changes when a required pin moves (the repo's commit changed it; the chip asks again).

### 2.4 Finding and installing (agent)
- An agent proposing a plugin through MCP (the proposal appears as a chip with the manual acceptance); an agent never installs.
- What an agent sees of installed plugins: every registry through MCP, checks it must satisfy, policies it is under, queries it may run, layouts in use, drivers available.
- A session's deviation when a policy denies a write: the denial as a Checks row with the check id; the agent's recovery.

### 2.5 Using, per kind (wireflows, on the shell)
- **facts**: a provider adds items and links (e.g. Helm values → config keys → the code reading them); where they appear (map rail or items, Reach, Ask, What's new on rebuild), origin tag, what happens when the provider errors or times out (rows marked, map unaffected).
- **query as check**: the finding chip, the Findings tab row with origin, the fix card template, the merge checklist line, CI `arch check` output, `allow` semantics for plugin checks.
- **query as overlay**: the toggle in the map tab bar, paint slots (outline · fill · badge · pill), the legend entry, stacking with the four core overlays, reserved colours.
- **query as inlay**: gutter glyph and hint (sett rule 12), never an inserted line.
- **query as Ask entry**: the catalogue, how the model learns the query exists, its witness format in the answer.
- **layout**: the picker, what changes (columns only), how areas and rules read under a new column rule, the override per unit stored in `.arch`.
- **policy**: Accept (shape → groups, gates, element attributes shown as badges and rows), the session (scheduler honouring groups, one group at a time in V1, gate runner, the Checks rows, denied writes), review (checklist lines), merge (gate), the manual path (gates become chips on a `you` session; `run gate` by hand), the fix-round budget and the question block when spent, the override record.
- **driver**: the picker in Accept · delegate ▾, hello/confinement, write attribution, what the session card shows for a different driver, driver-specific questions.

### 2.6 Observing and managing
- Origin tags on every contributed line (finding, checklist, map legend, inlay, Ask answer): the plugin's name, a click to its row.
- ⋯ per installed row: configure (a form from the manifest's settings schema, no custom UI), disable here · disable everywhere, require · recommend for this repo (a hunk in `.arch/plugins`), uninstall; what happens to rows and records when a plugin is disabled (not-evaluated vs removed).
- The status-bar count and what it opens.
- Errors: a crashing or slow plugin (timeouts per call, a budget per rebuild), how the core degrades (rows not-evaluated, a Checks row with the error, never a blocked map), and the chip that says so.

### 2.7 Records and CI
- Where plugin results live under `.arch/sessions/<id>/records` (gate outputs, judge verdicts, denials, overrides) as witnesses; retention; what review reads.
- `arch check` in CI: installs the lockfile headless, runs core and plugin checks, emits the merge-pane rows; what a missing private registry does.

### 2.8 Validation: the test-case plugins, designed just enough
Design each far enough to run through the wireflows above and find what the contract or the surfaces lack; do not build them.
1. **waves** (policy): groups by dependency, parallel sub-agents per group (V2; one group at a time in V1), gate = tests green + `arch check` + judge, fix-round budget 2, commit per element, the manual path with gates as chips. Confirm the session-card lanes and the Map badge + tint (no band).
2. **tdd-first** (policy): the test element before its covered element; the gate "red then green"; the denied write when production code comes first.
3. **commit-shape** (policy + query): one commit per element, message template with the `Arch-Element:` trailer, a finding in review otherwise.
4. **codeowners** (query as overlay + check): paint items by owning team; checklist line "owners of touched areas notified".
5. **helm-values** (facts): config keys as items with witnesses in `values.yaml`, linked to the code that reads them; an Ask query "which code reads `pool.size`".
6. **context-layout** (layout): columns by bounded context; proves the layout kind is not shaped for hexagon and layers only.
7. **codex-driver** (driver): a second driver; proves hello/confinement, write attribution and the stale-write guard hold for a driver that is not Claude Code.

Any case that needs a surface the shell does not have is a finding against the shell; bring it back rather than inventing a surface.

## 3. Deliverables

1. **The spec**: one document, sections mirroring §2.1–2.7, each contract stated as data (schemas, WIT excerpts, manifest fields, MCP methods), each behaviour as an invariant with the surface it uses, error and degradation paths included.
2. **Wireflows**, on the retained shell (reuse its CSS and helpers; every state is the real frame): author-and-sideload · find-and-install from a `+` door · accept a repo-required plugin · agent proposes · one per kind in use (facts, check, overlay, inlay, Ask, layout, policy across plan → session → review → merge, driver) · manage (⋯, disable, update) · degrade (error, timeout, not installed) · CI.
3. **The seven test-case sketches** with a one-paragraph verdict each: what the contract lacked, what the shell lacked, or "fit".
4. **The core hooks, final**: the list V1 must implement, each with its contract and the surfaces it feeds, updated from the plugins artifact after the wireflows.
5. **The not-pluggable list**, final, each with a sentence of justification.
6. **Open decisions** for the product chat, as a checklist.

## 4. Constraints
- Lean: anything a second real plugin can wait for is marked V2 and not designed now.
- No new surfaces: chips, pills, rows, the inspector, the Plugins tab, the Checks and Findings tabs, origin tags, the status-bar count. Verbs in bars or chips, never modals.
- sett components only; a component that does not exist is listed for sett, not improvised.
- Rust for arch-server and the SDK; TypeScript for the Theia side; the JSON/WIT contracts agreed first.
- Every contributed thing has a witness or is marked declared; every denial cites a check id that also applies to manual edits.
