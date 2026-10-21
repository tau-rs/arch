# arch · design record

**arch** is a desktop tool for Rust codebases, built as an Eclipse Theia product, in which a person and coding agents work on a repository through its architecture map. arch computes the map from rust-analyzer facts, keeps its own notes under `.arch/`, writes nothing into source, and lets you plan a change, delegate it to a session, follow it, review it and merge it, with the manual path always present. The full statement is `spec/arch-v1-spec.md` §1.

This repo is **the record**: the one place where the product's decisions live, with a git history. Every other repo links to it; none depends on it at build time.

## Rules

- A decision is taken in the product chat and written here as an ADR **before** any repo implements it. An implementation session that needs a decision not in `adr/` asks the product chat; it does not decide alone and it does not leave the question in code comments.
- The spec is amended, not forked: when an ADR changes a section, the section is rewritten and the ADR is referenced.
- The flow pages are the visual spec. A page is superseded by moving it to `flows/superseded/`, never by editing it in place; a new page is a new file.
- Plain text, no build step, no generated content. Diagrams are the HTML pages or Mermaid in Markdown.

## Repo map

```
arch-design/
  README.md                    this page
  roadmap.md                   V1 · V2 · V3 (spec §3), kept current
  spec/
    arch-v1-spec.md            the V1 specification; §13 holds the decisions of 2 Oct 2026
    vocabulary.md              the terms, with the C4 mapping
    keyboard.md                the keyboard map
    map-invariants.md          MAP-1…33 and CANVAS-1…2 as testable statements, each with its owner and check
  flows/                       the HTML pages, one file each, self-contained (open in a browser)
    shell.html                 the shell: one bar, rail, left panel, bottom panel, status bar
    plan-shell.html            plan and delegate, on the shell
    session-shell.html         session: follow · live · asks · deviation · gate · take over · done
    review-merge-shell.html    review and merge: glance · detail · remark · verdict · merge · merged
    daily-shell.html           daily: map and selection · read code · Ask · edit by hand · finding · What's new · commit
    map-focus.html             the map: anatomy, folding, navigation, overlays, edit at scale, invariants
    map-real-projects.html     the map on real projects (Zed, tokio, Bevy, Vector)
    v2-canvas.html             V2 canvas: zoom levels, tiers, sockets → ports, flow overlay
    superseded/                the earlier pages, kept for content not redrawn (see spec §2)
  adr/                         one file per decision: context · decision · consequences, numbered, dated
  handoffs/                    every handoff written so far, and the per-repo ones
```

## ADRs

Synced-to line for the other repos: **ADR 0037** (2026-10-21). Later decisions are added as 0038, 0039, …

| # | decision |
|---|---|
| [0001](adr/0001-store-sqlite.md) | Store: sqlite in `.arch/cache/`, plain text committed |
| [0002](adr/0002-keys-commit-hash-deltas.md) | Keys: facts by commit hash as per-file deltas |
| [0003](adr/0003-threads-and-records.md) | Threads and records: on the session's branch, archived to git notes at merge |
| [0004](adr/0004-areas-derived-no-source-annotation.md) | Areas: derived from the module tree, overrides only, no annotation in source |
| [0005](adr/0005-context-pack.md) | Context pack: the standard input to planner, sub-agents, judge and Ask |
| [0006](adr/0006-arch-init.md) | `arch init`: one commit, no questions |
| [0007](adr/0007-unit-in-v1.md) | Unit in V1: the main `[[bin]]` or the lib, and its closure |
| [0008](adr/0008-non-rust-ignored.md) | Non-Rust: ignored, except table names from `migrations/*.sql` |
| [0009](adr/0009-confidence-levels.md) | Confidence: `resolved` · `guessed` · `declared` |
| [0010](adr/0010-degrade.md) | Degrade: syntax-level facts marked guessed when rust-analyzer cannot type-check |
| [0011](adr/0011-worktrees.md) | Worktrees: `<parent>/<repo>-w<n>` |
| [0012](adr/0012-confinement-driver-hooks.md) | Confinement: the driver's pre-edit and post-edit hooks |
| [0013](adr/0013-judge.md) | Judge: same provider, fresh invocation, fixed prompt, structured verdict |
| [0014](adr/0014-concurrent-sessions.md) | Concurrent sessions: warn, show overlaps, never lock |
| [0015](adr/0015-restart.md) | Restart: resume each running session at its last turn boundary |
| [0016](adr/0016-commits.md) | Commits: agents commit through arch's tool, one per element, with trailers |
| [0017](adr/0017-push.md) | Push: agents never push |
| [0018](adr/0018-forge-github-first.md) | Forge: GitHub first, GitLab in V1.x, `Forge` trait from day one |
| [0019](adr/0019-conflicts-resolve-element.md) | Conflicts: a resolve element on the session's plan, fast gate |
| [0020](adr/0020-plan-drafts.md) | Plan drafts: in the cache until Accept; discard deletes |
| [0021](adr/0021-element-identity.md) | Element identity: content-addressed, `E3` as label, continuity by site then similarity |
| [0022](adr/0022-review-without-plan.md) | Review without a plan: works for any branch |
| [0023](adr/0023-settings-errors-secrets-packaging-telemetry.md) | Settings, errors, secrets, packaging, telemetry |
| [0024](adr/0024-repositories.md) | Repositories: arch · arch-app · sett · arch-fixtures · arch-design |
| [0025](adr/0025-direction-call-family-grain.md) | Direction: only call-family links carry it, with the grain of the column rule (left → right in layers, inward in hexagon) |
| [0026](adr/0026-budgets-first-index-memory.md) | Budgets: first index < 5 s cold at 1,200 items; `arch` < 500 MB RSS at 5,000 visible items; one benchmark per budget |
| [0027](adr/0027-arch-init-sides-rule.md) | `arch init` sides: entry → driving, I/O external → driven, otherwise domain; split one level on a driving/driven mix; override in `areas.toml` |
| [0028](adr/0028-entry-kinds-spawned-worker.md) | Entry kinds: a function spawned once at start-up that loops for the life of the process is an entry (`spawned worker`); `resolved` when spawned in `main`, `guessed` otherwise |
| [0029](adr/0029-arch-init-order-rule.md) | `arch init` order: alphabetical by area name within a side (bytes, ascending); one area per top-level module, a single file included; grouping is a hand override; after `init` the file is the order |
| [0030](adr/0030-analysis-inputs-outside-the-tree.md) | Inputs outside the tree: target triple, toolchain and build-script output are in the package id; the target is this machine's unless `areas.toml` pins it; features are the unit's defaults |
| [0031](adr/0031-items-and-item-ids.md) | Items and ids: what rust-analyzer calls an item, methods included; variants and fields named in `Link.member`; a method's id sits under its impl's header as written, a non-lib target is marked `[bin:<name>]` |
| [0032](adr/0032-rules-allows-format-five-lints.md) | `.arch/rules` and `.arch/allows`: TOML, schema version 0 as smallsvc writes it, a level per lint; the five lints are `god-module` · `cycle` · `leaky-port` · `speculative-abstraction` · `unresolved-dyn` |
| [0033](adr/0033-link-kinds-and-families.md) | Link kinds: 21 in four families, call · type · data · structure, plus `refers-to`, as schema version 0 ships them; `routes` and `queues` are kinds 20 and 21; `calls` · `calls-out` · `hands-off` · `listens-to` · `routes` carry a direction |
| [0034](adr/0034-arch-api-wire-contract.md) | arch-api wire contract: `schemas/arch-api.json` (OpenRPC 1.3, `x-arch-events`) pinned by commit; `arch serve` on a per-repo unix socket, one JSON-RPC message per line; `initialize` returns engine and schema versions; the schema version is semver |
| [0035](adr/0035-what-the-five-lints-check.md) | What the five lints check: `god-module` an area over 40 % of the unit's lines; `cycle` areas that depend on each other, reported on the cheapest cut; `leaky-port` a port's signature naming adapter technology; `speculative-abstraction` a trait with one implementor (a port: and no test implementor); `unresolved-dyn` a port called with no implementor or several and none wired |
| [0036](adr/0036-engine-binary-name.md) | Engine binary: stays `arch`; a candidate is the engine only if `--version` prints `arch <semver>` first; the packaged app runs its bundled engine before `PATH` |
| [0037](adr/0037-open-area-rows.md) | Rows in an open area: one per top-level item; `impl` blocks and methods fold into their type's row (`Item.self_type`), `private` loose items into their module's row; rows start closed, open on demand |
## Flow pages

| page | what it is |
|---|---|
| [shell](flows/shell.html) | the retained shell and its invariants SHELL-1…4, LEFT-0…8 |
| [plan](flows/plan-shell.html) · [session](flows/session-shell.html) · [review and merge](flows/review-merge-shell.html) · [daily](flows/daily-shell.html) | the four flows on the shell (spec §6) |
| [map focus](flows/map-focus.html) · [real projects](flows/map-real-projects.html) · [v2 canvas](flows/v2-canvas.html) | the map (spec §5) |
| `flows/superseded/` | ask · base · findings · firsts · git · merge · onboarding · plan · review · session · shell-flows · sync; still cited by spec §2 for the Update dialog, the fix card, judgement rows and reconcile |

## Handoffs

| file | for |
|---|---|
| [handoff-arch-design.md](handoffs/handoff-arch-design.md) | this repo |
| [handoff-arch.md](handoffs/handoff-arch.md) | `tau-rs/arch`, the engine |
| [handoff-arch-app.md](handoffs/handoff-arch-app.md) | `tau-rs/arch-app`, the Theia product |
| [handoff-arch-fixtures.md](handoffs/handoff-arch-fixtures.md) | `tau-rs/arch-fixtures`, test data and invariants |
| [handoff-sett-rebase.md](handoffs/handoff-sett-rebase.md) | `tau-rs/sett`, the design system: rebase on the retained shell |
| [handoff-design-system.md](handoffs/handoff-design-system.md) | the design-system side-work (done; sett exists) |
| [handoff-plugin-system.md](handoffs/handoff-plugin-system.md) · [handoff-plugin-system-spec.md](handoffs/handoff-plugin-system-spec.md) | the plugin system side-work (designed; V2) |
| [handoff-visualizer-poc.md](handoffs/handoff-visualizer-poc.md) | the visualizer proof of concept (done) |

## The other repos

| repo | is | states |
|---|---|---|
| [`tau-rs/arch`](https://github.com/tau-rs/arch) | the engine: facts, views, server, tool layer, drivers, forge; one Cargo workspace | the ADR it is synced to |
| [`tau-rs/arch-app`](https://github.com/tau-rs/arch-app) | the Theia desktop product, consuming `@tau-rs/sett` | the ADR it is synced to |
| [`tau-rs/sett`](https://github.com/tau-rs/sett) | the design system: tokens, Lit `sett-*` elements, Storybook; `DESIGN.md` wins on look, the ADRs on behaviour and vocabulary | the ADR it is synced to |
| [`tau-rs/arch-fixtures`](https://github.com/tau-rs/arch-fixtures) | pinned real repos, golden facts, invariant and performance checks | the ADR it is synced to |

## Sync with the other repos

Weekly, or at every milestone of a repo: an implementation session reads `adr/` newer than its last sync and `roadmap.md`, applies what changed, and files its own findings (a surface the shell lacks, a fact the analyzer can't produce, a component sett refuses) as issues here labelled `from:<repo>`. The product chat triages those into ADRs.
