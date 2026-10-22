# arch · V1 specification · handoff to implementation

Oct 2026. Consolidates the product chat, the sett design system, the visualizer PoC, and the shell and plugin side-chats. Plugins are designed but out of V1 (§8). Where this document and an artifact disagree, this document wins; where it is silent, the artifact listed in §2 is the spec.

## 1. What arch is

A desktop tool for Rust codebases, built as an Eclipse Theia product, in which a person and coding agents work on a repository through its architecture map. arch computes the map from rust-analyzer facts, writes almost nothing into source (one `//! @arch area:` line per file), keeps its own notes under `.arch/`, and lets you plan a change, delegate it to a session, follow it, review it and merge it, with the manual path always present.

Principles (normative):

- **P-1 · two doors, agent door first.** Every action has a manual path and an agent path, both visible; the agent door is first and bold, the manual door is always there and never privileged; the "me first" setting swaps order and weight. Never a single "let the AI" button, never a hidden manual path.
- **P-2 · reader, not owner.** arch watches the repo and every worktree and rebuilds from files. An edit in arch is not a special case. Nothing pops up, nothing blocks; state is a chip, a pill, a row, the frame's colour, a What's new line.
- **Witness rule.** Every fact shown has a source (file:line, a contract, a tool output). The host re-reads every locator; a plugin that contributes facts contributes witnesses.
- **The map never re-layouts for state.** Positions inside a unit come from facts; everything else paints.
- **Nothing is written before you say so.** Accept, Keep, commit, merge are the writes; discard leaves no trace.

Vocabulary: repo · unit · area · item · link · finding · witness · rule · lint · allow · plan · element · group · gate · session · sub-agent · scope · Focus · Lock · take over · remark · resolution record · What's new · port · rail · node · tier · kind. Mapping to C4 for architects: repo = context, unit = container, area = component, item = code.

## 2. Artifacts (the visual spec)

Flows, on the retained shell (these supersede the earlier flow pages):

| flow | artifact |
|---|---|
| Plan and delegate | https://claude.ai/artifact/5yqj554jto8B96bvepuAra |
| Session (follow · live · asks · deviation · gate · gate failed · take over · done) | https://claude.ai/artifact/Eu2VyYvcjsDziTCoB271Dc |
| Review and merge (glance · detail · remark · plan delta · verdict · merge · merged) | https://claude.ai/artifact/1MxQnkAfsxh61hAQWkvkqU |
| Daily (map and selection · read code · Ask · edit by hand · finding · What's new · commit) | https://claude.ai/artifact/Uq9zaFSn3WDbDZSXJ2XpF6 |

Shell and plugins (side-chat, decided):

| part | artifact |
|---|---|
| The shell: one bar, rail, left panel, bottom panel, status bar | https://claude.ai/artifact/KENuEgpvSjX4p7jRTGxebn |
| Shell wireflows (supervise, read, work by hand, answer and gate, panel) | https://claude.ai/artifact/4CsJdkWyWPMvZAWep9pezB |
| Plugins: marketplace, extension points, contracts (V2) | https://claude.ai/artifact/JGtK8hmBmkakcvRkwqWkmy |
| Plugins wireflow (V2) | https://claude.ai/artifact/CVJt77dw8uCTQgYvkmsNYF |
| Sessions wireflow (Changes list, review inputs) | https://claude.ai/artifact/99rubFaMoZ38R4rJNu1zjt |

Map (standing):

| part | artifact |
|---|---|
| Map focus (anatomy, folding, navigation, overlays, edit at scale, invariants) | https://claude.ai/artifact/PTACFVEdRRHB4uYKW7dgTr |
| The map on real projects (Zed, tokio, Bevy, Vector) | https://claude.ai/artifact/PQoTSQ8JU6avRADKDRZjwq |
| V2 canvas (zoom levels, tiers, sockets→ports, flow overlay) | https://claude.ai/artifact/K8ur1qKoCYb3up5c1T8Cwa |
| Visualizer PoC (ripgrep, zero2prod, Zed) | https://claude.ai/artifact/VfzitVMbqi526zZesNy47i |

Still useful from the earlier set, for content not redrawn: git flow (Update dialog [Rebase] · [Merge] · [Stash] · [Shelve] · [Keep], conflict resolution 7a/7b, resolution record) https://claude.ai/artifact/DYLTmUXFxat7f3vE27jYZk; findings flow (fix card, allow, findings pane) https://claude.ai/artifact/Qcst7B3a2PWsPzWbZSEggN; ask flow (judgement with resolve rows, can't-compute) https://claude.ai/artifact/4hmAvh52j5hfGoWhkZ1fHy; sync flow (reconcile with impact, stale elements, re-plan · drop · revert) https://claude.ai/artifact/CGrQsXSw9ghaN8GmQkQMEJ. Onboarding and firsts pages exist and are out of V1.

Design system: `tau-rs/sett` (DESIGN.md is normative; tokens DTCG; Lit `sett-*` elements; Storybook with a11y gate). arch consumes `@tau-rs/sett`, loads `sett.css` and the element bundle in Theia, keeps `design/theme.json` for semantic overrides only, composes sett's Storybook, symlinks `skills/sett`.

## 3. Roadmap

| | scope | contents |
|---|---|---|
| **V1** | one unit per repo (an app, a service, a CLI, or a single library; private helper crates folded in) | everything in §5–§9; hexagon **and** layers; externals at the interface tier (ports for what the repo touches); framework-held entries; whiteboard tools on the unit sheet; the shell; `.arch/` written by CLI or by hand (no onboarding funnel); **no plugin system**: the groups and gates of the plan, the checks, the driver are core and fixed |
| **V2** | units · plugins | multi-crate repos: unit grouping proposed and kept in `.arch/units`; repo board with free placement and portals; hierarchical fold (unit › crate › module › item, modules as default sub-areas); profiles; trait families; sub-areas by kind; sibling plans; onboarding gains a units step; **the plugin system** (five kinds, marketplace, lockfile, PLG-1…10) with the first policies (waves) and providers |
| **V3** | system | multi-repo board; edges from contracts, topics, shared crates, config, or declared and marked; flow overlay across repos; plans spanning repos; runtime traces as witnesses |

Present in the V1 data model so V2/V3 are additions: a scope id on every item; the public surface (ports) computed; `.arch/board` schema (empty); zoom depths named; commits as facts; groups and gates in the plan schema (produced by a fixed core shaper in V1, by policies in V2); an `origin` field on every finding, check and overlay row (always `core` in V1).

## 4. The shell (decided; invariants SHELL-1…4, LEFT-0…8)

- **One bar** (44 px): `arch · orderly › [scope selector] … chips … Ask ⌘K`. The scope selector is the crumb and reads `main`, `w1 · refund flow` in its colour, `you · fix-pool-size 🔒`, or `plan · refund flow` (amber). Only chips that carry a verb (answer, open, take over, commit, merge). Nothing written twice.
- **Rail** (56 px): glyph + horizontal label, **Sessions · Files · Findings**, Sessions on top with the asks-you badge, Findings with a new-blocking badge; a 2 px bar in the session colour on the active label when a session is focused. Never hides, never icons alone, never vertical text.
- **Left pane** (260–280 px): the active rail view; a scope line at its top (indicator, never a control). Folds with ⌘B except while reading.
- **Center**: the Map pinned first; files as tabs in the scope's version, tab underlined in the session colour; the plan's intention bar sits above the Map in a plan scope. The Map and the plan are never displaced.
- **Inspector** (330 px): about the selection: item (witness, callers, code), file preview or diff, session card, sub-agent thread, planner thread, fix card, commit, checklist. Folds to a 28 px handle (⌘⇧]).
- **Bottom panel**: Findings · Checks · Terminal · What's new only. ⌘J; closed is a 30 px strip with counts.
- **Status bar** (26 px): counts and states, each a click to its owner; no verbs.
- **The frame**: 3 px border; violet gradient = agent working on the scope; amber pulse = waits for you; blue still = your uncommitted work; red still = collision or deviation; amber still = planning. Only the border moves (plus one 700 ms ring on a changed map item; camera fit 300 ms; tier swap 150 ms; flow dash on the selected unit's edges). `prefers-reduced-motion` stills all.

**Scope and Focus.** The scope is the worktree the shell is about: main, a focused session's worktree, or a detected `you` session. Looking is not jumping: a click selects (card on the right, items lit); Focus (row button, card button, ↩, a chip, the bar's selector) makes a session the scope and isolates the Sessions view on it; Esc unfocuses; Lock pins a focus. Switching rail views never changes the scope; Files always shows the focused worktree.

**Sessions view**: every session grouped Planning · Yours (detected, lockable) · Needs you · Running · In review · Done; a session unfolds into groups › sub-agents (element, state) › files edited; a Changes row per session; `+ new session · delegate` at the end. **Files view**: the focused worktree in two projections, Directory and Layers; one selection shared with the Map, both ways; a 3 px bar in the session colour on files an agent is writing. **Findings view**: the scope's findings grouped by rule, blocking rules first, filtered to what the scope introduced; the bottom panel's Findings tab is the same set as one table (level · finding · rule · witness · origin). A row in either selects the same way: the Map goes to the item with the findings overlay, the inspector shows the fix card.

**Work by hand.** A write the watcher cannot attribute to an agent creates or updates a `you · <worktree>` session under Yours; the git chip counts the changes; commit is one click from the chip (message prefilled from the diff or the plan element with its `Arch-Element:` trailer); `Delegate the rest` turns what is left into a plan.

## 5. The map (V1 subset of MAP-1…33, CANVAS-1…2; the full list with owners is `map-invariants.md`)

- Positions inside a unit come from facts and are byte-identical between saves for unchanged items; no state re-layouts (MAP-1, 3, 24). V1 has one unit, so there is nothing to place by hand yet; the board schema exists.
- Two column rules, chosen from the presence of an entry, overridable and declared in `.arch`: **hexagon** (driving → domain → driven → externals) and **layers** (public API left, internals, leaves right). Direction (MAP-17, under MAP-26): only directed links (`calls` · `calls-out` · `hands-off` · `listens-to` · `routes`, [ADR 0033](../adr/0033-link-kinds-and-families.md)) between two columned items carry it, with the grain of the rule, left → right in layers, inward to the domain in hexagon; a call against the grain is drawn as a smell; `implements`, type references and links to rails never are ([ADR 0025](../adr/0025-direction-call-family-grain.md)).
- Fold, never shrink: one label size; above ~80 visible items areas fold to chips with counts; `areas / items` is a switch; the fold keeps the footprint with a floor at 0.7 of the opening scale (MAP-4 amended, MAP-6 dropped).
- Rows in an open area: one row per top-level item; an `impl` block and its methods fold into their type's row, a trait's methods into the trait's row, a `private` item loose in a module into that module's row; every row starts closed and opens on demand; links, findings, overlays and Reach that land on a folded item land on its row with a count; a folded item is not a visible item (MAP-33, [ADR 0037](../adr/0037-open-area-rows.md)).
- Open in place: repo › unit is not a level change (trivial in V1); code opens as a tab only by double-click or ↩ (MAP-5 reworded).
- Externals are the rails: ports of kinds rpc · http · cli · topic · crate · sql · pub · redis · fs · tty · declared on the unit's flat sides, sections in fixed order (platform services · third-party · events · data stores · os · libraries · unresolved); a neighbour's matching port is wired straight; edges dock to ports and detour rather than move anything; no outbound column (MAP-31, 32).
- Node tiers by on-screen width, same thresholds everywhere (mini 110 · chip 380 · card 760 · open 900 px, hysteresis 40), cross-fade on swap, camera moves, the node never resizes itself (CANVAS-1).
- Four overlays, toggles in the tab bar: sessions (colour per session, shades per sub-agent, outline), plan (amber dashed elements and links, group badges), findings (red pills on links, ⚠ on items), delta vs main (unchanged faded, added drawn, removed ghosted). Outline = session, fill = plan or finding. No overlay groups items spatially (the wave band is a badge + tint per item).
- Unresolved paths (dyn, spawn) folded to one pill per item; Reach expands them (MAP-16/28).
- Navigation: ⌘P finds items, files, areas, rules; ↩ selects and opens where it is; focus fades outside the Reach ring; `f` fit; arrows walk links; minimap only when the map exceeds the viewport; Esc goes tab → unit → focus → level.
- Editing areas: whiteboard toolbar (select V · pan H · area A · lasso L · rename · delete · undo · redo · framer-places-the-rest · fit), a selection box with handles and the action pill attached to it, an Unplaced tray; drawing an area is constrained to one column; all edits undoable until Keep; Keep is one change in Changes.
- Rendering: DOM (sett ADR 0001); performance budgets owned by arch: first index of a 1,200-item repo < 5 s cold, first paint of a 1,200-item repo < 2 s, fold/expand < 100 ms, recompute of one file < 500 ms, 60 fps pan/zoom at 5,000 visible items, `arch` process < 500 MB RSS at 5,000 visible items; one benchmark per budget ([ADR 0026](../adr/0026-budgets-first-index-memory.md)).
- Trait families, profiles, sub-areas by kind and sibling plans are V2 (MAP-27, 28, 29; PLAN-1).

## 6. Flows (what each screen is; the artifacts hold the states)

**Plan and delegate.** A plan is a session that has not run. Doors: the plan chip, `Plan` on a selected item, `make it so` from Ask, `Delegate the rest` from a `you` session, `+ new session · delegate`. A row appears under Planning and is focused (scope `plan · <name>`, amber). The intention bar sits above the Map; ↩ drafts: the planner draws elements in amber and lists them as rows; it asks what it cannot decide, with options that say what each changes. Shaping happens on the Map (rename, drag, ✕, ＋) or in the thread; every reply ends with `changed · what` or `no change`; the core shaper groups elements by dependency with one gate per group. The fixed bar: **Accept · delegate ▾** (primary) | **Save plan** | discard. Accept creates the branch, the worktree and `.arch/sessions/<id>/plan` with the thread; delegate moves the row to Running and starts the driver; Save plan moves it to Yours as a locked `you` session, elements as gutter marks and hints (no inserted lines). The planner's thread is the first part of the session's thread.

**Session.** Follow: Sessions view isolated on it, card in the inspector (plan, groups as lanes, files, git) with the verbs bar (words: pause · stop; paused: resume · take over · stop) and the composer. Live: the file the sub-agent edits opens as a tab, gutter marks in its colour; you can type; the stale-write guard rejects its next write to a file you changed until it re-reads; it is told What's new at its next turn boundary; nothing pauses on a human edit. Asks: amber pulse, chip with `answer`, batched questions with options; `later` leaves it waiting. Deviation: a write outside the element's scope is denied in the tool layer by the core rule, reported with the agent's reason and three typologies (back on the plan · update the plan · not this change); Discuss is not a decision. Gate: a group ends, the gate runs (commands, `arch check`, judge), rows on the Checks tab with output as witness; `step in` opens that tab. Gate failed: a fixed fix-round budget (2) runs; when spent the session stops and asks (one more round with a hint · take over · accept as is with a recorded override · re-plan). Take over: pause, then take over one element; hand back is written in the composer, lands in the thread as your message with its changed line, the agent resumes; the plan stays the session's. Done: handover message; create MR drafts the description from the plan and the thread.

**Review and merge.** Glance: Map delta plus the checklist (plan realized, gates and judge, findings, pipeline, remarks, current with main, files viewed · optional); `approve · merge` or `review in detail`. Detail: the session's Changes list (unstaged · staged · commits, status letters, viewed ✓), hunks in one scroll as a tab (j k · v · r · show on map), one selection with the Map. A remark is `comment · no change needed` or `ask for a change`; a change is a plan element with the remark as intention and the hunk's item as site, realized in the session or by hand, never in the review tab; the hunk returns to unviewed on commit. Verdict when every line but approval is ✓: approve · request changes (a remark that asks) · later. Merge: the same checklist, the how (squash or merge commit from the forge's default, delete branch, archive session), the gated button that says what it does; pipeline on the Checks tab; arch never merges on its own. Result in the checklist shape with the two pills **merged** (forge fact) and **archived** (arch fact: branch and worktree gone, plan and threads kept, restorable); the row moves to Done.

**Daily.** Map and selection (Layers projection, one selection, inspector about it, Code | Reach). Read code (tab, Map reduced in the inspector, witnesses as highlighted spans, findings as wavy underlines, counts and hints in pills; no inserted lines). Ask (inspector thread; queries run first, one line each; the answer names items as chips and cites witnesses; the Map focuses what it names; judgements labelled with facts and resolve rows; can't-compute offers the nearest queries; `make it so` → plan). Edit by hand (editor, `you` session detected, blue frame, chip counts changes). Finding (chip, Findings badge and tab, pill on the link, fix card with the verified proposal and both doors; allow is person-only, one entry in `.arch/allows`). What's new (Map pulses once, chip, inspector doors: map delta · findings ± · unplaced · a session told at idle · tests; same lines in the panel tab). Commit from the chip (ready commit in the inspector; pick hunks; then stay on main or push and open an MR); behind main as a chip with both doors; the Update dialog with technical names from the git flow.

**Sync.** One watcher over the repo and its worktrees; any change (yours, a pull, another editor) enters the same path: facts recompute for changed files, findings re-run on the diff, the plan reconciles with an impact analysis (the element rewritten ≠, dependent elements **stale**, sub-agents affected, hunks re-flagged), each stale element re-planned, dropped, or the edit reverted; a running session gets the What's new at its next step with the stale-write guard protecting your edit; a new item without an area line is guessed, drawn faintly, listed in the Unplaced tray, placed by one click or by the framer's thread.

## 7. `.arch/` and the fact model

```
.arch/
  areas.toml       overrides only: path patterns → area, side, order, main bin (areas derive from modules)
  areas/<name>.md  optional description per area, read into the context pack
  rules            dependency rules (subject · must not · targets · level) and lint settings (the lints: ADR 0035)
  allows           allowed sites, keyed by site, person-only
  sessions/<id>/   plan.toml (elements with content-addressed ids, groups, gates), thread.jsonl,
                   records/ (gate outputs, judge verdicts, denials, overrides, resolution records)
                   — on the session's branch; moved to refs/notes/arch on main at merge
  cache/           sqlite store (gitignored): facts by commit hash, view caches, plan drafts
  board            (V1: empty) free positions of units
  plugins · units  (V2)
```

Facts the analyzer must produce in V1 (from rust-analyzer and cargo): crates and targets; items with kind, file, line, visibility, re-export; links of the 21 kinds in four families plus `refers-to` ([ADR 0033](../adr/0033-link-kinds-and-families.md)), resolved or unresolved with the reason; entries including framework-held ones (table: axum, actix, tonic, bevy App, lambda; generic rule: the entry calls into a dependency that calls back) and spawned workers (a function spawned once at start-up that loops for the life of the process; [ADR 0028](../adr/0028-entry-kinds-spawned-worker.md)); public surface per crate (pub items, pub use, registered routes/rpcs/topics); externals with the subset of their API touched; `route → handler` with the middleware stack (`routes`); `shared table used as a queue` (who inserts, who dequeues; `queues`); facades (only `pub use`); tool-bin and example detection; commits on the branch as facts (author, trailer, files, plan element). Every agent write passes through the tool layer with the content hash of the last read.

## 8. Plugins (V2, decided; out of V1)

The plugin system is designed and decided (five kinds, WASM and subprocess runtimes, marketplace, `.arch/plugins` lockfile, acceptance, `not-evaluated` rows, PLG-1…10, the core hooks list) in the plugins artifacts and the spec handoff; nothing of it ships in V1. V1 keeps the three seams the design relies on so V2 is an addition: the tool-layer pipeline (stale-write guard → veto → attribution) with a single core veto rule (an element's sub-agent may only write its element's files), the query registry with core checks only and an `origin` on every row, and the plan schema with groups and gates filled by a fixed core shaper (groups by dependency, one gate per group: `arch check` and the project's test command). No plugin host, no registry client, no Plugins tab, no `.arch/plugins` in V1. Theia's workspace trust is off: with no plugin host, tasks or debugger in V1 it would hold back nothing, and the engine runs the repository's build scripts and proc macros on open regardless; opening a repository in arch is trusting it, as with `cargo build` ([ADR 0038](../adr/0038-workspace-trust-off.md)).

## 9. Architecture

`arch-rust` (ra_ap_* facts, cargo metadata, watcher) → `arch-facts` (local store, sqlite or sled; commits as facts) → `arch-views` (column rules, folding, overlays, impact analysis) → `arch-server` (query API, MCP, drivers, tool layer with the stale-write guard and the core veto, scheduler, forge read) → clients (Theia UI consuming sett; CLI `arch check` for CI; MCP for agents). Everything the UI shows is reachable through MCP, so an agent and a person see one view. Forge: GitLab first (MR, pipeline read, merge with the project's squash setting). Agent driver: Claude Code first, behind `AgentDriver`.

## 10. Requirements index

Earlier UX-1…111 and MAP-1…16 are in the previous session summaries and pages; those still valid are absorbed above. From this session: UX-112…150 (hints not modals; Actions chips ordered by what unblocks; conflicts agent-first with manual fallback and a resolution record; git verbs as links; wireflows with both doors; brainstorm from a finding; resolutions in the finding's row; Accept always creates branch/worktree/plan and delegates by default; the planning thread kept and handed to the session; deviation typologies; hand-back as a note; session card pinned → now the Sessions view; one thread per session; merge gated by the checklist; CI displayed never run; archived keeps plan, threads, remarks, decisions; result reuses the checklist shape; merge strategy from the forge; merged ≠ archived; readiness is one percentage (onboarding, out of V1); the map's whiteboard tools; selection as a box with actions attached; What's new as one chip and pane; the session never paused by a file change; reconcile computes impact and finishes when every stale element is re-planned, dropped or reverted). Shell: SHELL-1…4, LEFT-0…8. Map: MAP-1…32 as amended in §5, CANVAS-1…2, each with its owner and check in `map-invariants.md`. Plugins: PLG-1…10. Principles: P-1 (agent door first), P-2 (reader not owner), the witness rule.

## 11. Out of V1

Onboarding funnel and firsts; the plugin system entirely (host, registry, lockfile, Plugins tab, policies, providers, layouts, extra drivers); units, board placement, profiles, trait families, sub-areas by kind, sibling plans (V2); multi-repo, cross-repo edges, flow overlay, runtime traces (V3); parallel groups in the scheduler.

## 12. Open items carried into implementation

- sett: `group` row kind for session-card lanes; the activity rail (Sessions · Files · Findings) as a component; layers column tints; (Plugins tab components deferred to V2) the layers direction (public API left) in the map stories; the PoC's fixture data exported.
- Fact model: measured sizes of the four real repos replace the approximations.
- V1 onboarding substitute: `arch init` CLI that writes `.arch/areas` from modules and `.arch/rules` from a template, so a repo without `.arch/` is usable on day one.

## 13. Decisions of 2 Oct 2026 (engine and data; supersede earlier text where they differ)

1. **Store**: sqlite in `.arch/cache/` (gitignored); committed `.arch/` content is plain text; `arch-facts` owns the schema.
2. **Keys**: facts keyed by commit hash as per-file deltas with content hashes; branches are pointers; a worktree-state hash keys uncommitted work; rebases reuse deltas by file hash.
3. **Threads and records**: on the session's branch under `.arch/sessions/<id>/` while it lives (`thread.jsonl`, `plan.toml`, `records/`), moved to a git notes ref on main at merge (the archive). The thread is arch's own (filtered driver stream plus the driver's session id and transcript path as pointers); records are gate outputs, judge verdicts, denials, overrides, resolution records.
4. **Areas**: derived from the module tree; `.arch/areas.toml` holds only overrides (path patterns → area, side, order, main bin); optional `.arch/areas/<name>.md` descriptions; **no annotation in source** (the `//! @arch area:` line is dropped; Keep writes `.arch/areas.toml` only).
5. **Context pack**: the standard input to planner, sub-agents and judge: the unit's map as text (areas, sides, ports, entries, cross-area links), rules, open findings, area descriptions, the element's position and what it may call. Also what Ask reads.
6. **`arch init`**: writes `areas.toml` (computed sides and order; the sides rule is [ADR 0027](../adr/0027-arch-init-sides-rule.md), the order rule is [ADR 0029](../adr/0029-arch-init-order-rule.md)), `rules` from the template (domain must not depend on driving or driven; externals only from driven; five lints on), the gitignore line; one commit; no questions.
7. **Unit in V1**: the main `[[bin]]` (or the lib) and its closure; other bins and examples "not analyzed" in the status line; `areas.toml` can name the bin.
8. **Non-Rust**: ignored, except table names from `migrations/*.sql`.
9. **Confidence**: `resolved` · `guessed` · `declared`; a finding on a guessed link warns, never blocks; on declared it can't gate a merge.
10. **Degrade**: a crate rust-analyzer can't type-check falls back to syntax-level facts marked guessed; shown (lightest ink, status-bar state, What's new line, reason on the Checks tab).
11. **Worktrees**: `<parent>/<repo>-w<n>`, recorded in the session, configurable.
12. **Confinement**: the driver's pre-edit hook calls arch (stale-write guard, element-scope veto), the post-edit hook attributes; the only confinement in V1. Verify on day one that `--settings` hooks fire under `--bare`.
13. **Judge**: same provider, fresh invocation, fixed prompt, structured per-element verdict with reason; may not propose code; overridable with a recorded reason; second provider as judge is V2.
14. **Concurrent sessions**: warn at plan time when a site is in another session's plan; show overlaps; never lock.
15. **Restart**: scheduler state in the session record; resume each running session at its last turn boundary with a "restarted" message.
16. **Commits**: agents commit through arch's tool: Conventional Commits `type(area): summary`, body from the intention, trailers `Arch-Element`, `Arch-Session`, `Co-authored-by`; one commit per element; direct `git commit` denied to agents; your commits free (trailer prefilled when attached); merge strategy from the forge.
17. **Push**: agents never push; push on create PR or on your click; CI on the forge only.
18. **Forge**: GitHub first (PR, checks, merge with the repo's allowed strategy), GitLab in V1.x; `Forge` trait from day one; UI words from the adapter.
19. **Conflicts**: a *resolve element* on the session's plan with both intents in context; fast gate (tests, check, one closed judge question, no fix-round loop); running sub-agents paused then told; manual door unchanged.
20. **Plan drafts**: in the cache until Accept; discard deletes.
21. **Element identity**: content-addressed `sha256(session · intention · site)[:8]`, `E3` as display label; continuity by site then similarity then a question in the thread; `plan.toml` owns the element; nothing in source.
22. **Review without a plan**: works for any branch; `plan · none · hand-made branch`; a remark asking for a change creates the first element.
23. **Settings**: a Settings tab (this repo · you · appearance). **Errors**: a status-bar state per subsystem plus a Checks row; never a modal; last good view kept. **Secrets**: OS keychain via Theia, never in `.arch`. **Packaging**: Theia desktop app spawning `arch` as a separate binary; `arch` CLI = `init · serve · check · mcp · hook`. **Telemetry**: none.
24. **Repositories**: `arch` (engine, one Cargo workspace), `arch-app` (Theia), `sett` (design system), `arch-fixtures` (pinned repos, golden facts, invariant checks), `arch-design` (spec, pages, ADRs). See the per-repo handoffs.
