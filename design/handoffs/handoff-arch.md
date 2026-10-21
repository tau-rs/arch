# handoff · `tau-rs/arch` · the engine

Rust. One Cargo workspace, eight crates, one binary. Everything that computes, stores, schedules or talks to agents and the forge. No UI. Read `arch-design/spec/arch-v1-spec.md` (§1, §5–§9, §13) and the ADRs first; this document is the engineering brief.

## 1. Layout and dependency direction

```
arch/
  Cargo.toml                workspace
  crates/
    arch-facts              model · sqlite store · .arch formats · event types        ← depends on nothing of ours
    arch-analyze            watcher · cargo · git · rust-analyzer → fact deltas       → facts
    arch-views              layouts · fold · Reach · overlays · findings · impact ·    → facts
                            checklist · What's new
    arch-session            plan · scheduler · gate runner · judge · context pack ·   → facts, views, driver, forge
                            thread · resolve element
    arch-forge              Forge trait · github adapter                              → facts
    arch-driver             Driver trait · claude-code adapter · hooks · MCP tools    → facts
    arch-api                JSON-RPC + MCP surface, one method set                    → all of the above
    arch-cli                `arch` binary: init · serve · check · mcp · hook          → api
  fixtures/                 pinned pointer (submodule or lockfile) to tau-rs/arch-fixtures
  docs/                     crate-level design notes; links to arch-design ADRs by number
```

Arrows go one way. A PR that adds a reverse dependency is wrong by construction; add a `cargo deny`-style check in CI (a small script over `cargo metadata`) that fails on it. This is the project eating its own rule.

## 2. Crate contracts

**arch-facts.** Types: `Item {id, kind (fn · struct · enum · trait · impl · mod · macro · const · static · type-alias · union), file, span, crate, module path, visibility, reexported, flags (entry · unsafe · drop · generated · cfg)}`, `Link {from, to, kind (the 21 kinds in four families + refers-to), confidence (resolved · guessed · declared), witness, flags (declared-or-shape · read-or-write · compile-time · unsafe)}`, `Port`, `External {name, kind, touched surface}`, `Entry {item, framework?}`, `Table`, `Commit {hash, author, trailers, files, element?}`, `Session`, `Plan {elements with content-addressed ids, groups, gates}`, `Record`. Store: sqlite under `.arch/cache/`, facts keyed by commit hash as per-file deltas with content hashes, branch → head pointers, a worktree-state hash for uncommitted trees, view cache tables. `.arch/` readers and writers: `areas.toml`, `rules`, `allows`, `sessions/<id>/{plan.toml, thread.jsonl, records/}`, the notes archive (`refs/notes/arch`). Event types for the bus. Golden-file tests against `arch-fixtures`.

**arch-analyze.** Watcher over the repo and its worktrees (debounced; writes made through arch's tool layer are tagged so they don't loop; an unattributed write is a `you` event). Cargo reader: targets, main-bin selection (first `[[bin]]` or lib; `areas.toml` override), closure, tool bins and examples marked not analyzed. ra_ap integration producing: items, links with kind and confidence (resolved from the type-checked view: calls, calls port, depends on port, implements, refines, inherits, uses type, holds, reads, matches on, tests, re-exports, expands, decorates, constructs; guessed from patterns: hands off, listens to, calls out, wires), entries including framework-held (table: axum, actix, tonic, bevy App, lambda; generic rule), public surface (pub items, pub use, registered routes/rpcs/topics), externals with touched surface, `route → handler` with middleware, table names from `migrations/*.sql` and the `shared table used as a queue` edge, facades. Syntax-only fallback per crate when type-check fails, all facts guessed, reason recorded. Git reader: pointers, merge base, commits as facts with trailers. Budget: one file recompute < 500 ms; first index of zero2prod < 10 s. This crate is the hard part; build it against the golden facts first.

**arch-views.** Pure functions over the store, cached per branch: column rules (hexagon from entry; layers: public API left, leaves right; "uses" left → right in both), positions (byte-identical for unchanged items), fold levels and footprint rule, Reach, the four overlays, findings (dependency rules + five lints as queries; `allows` applied; confidence rule: guessed → warns, declared → never gates), impact (≠, stale dependents, affected sub-agents, hunks), checklist, What's new folding per branch, the review view (works with `plan · none`). Snapshot tests on fixture facts; no analyzer in this crate's tests.

**arch-session.** The state machine (Planning → Running → asks | deviation | gate | gate-failed → Done → In review → Merged · Archived). Plan: drafts in the cache, Accept writes branch + worktree (`<parent>/<repo>-w<n>`) + `plan.toml` + thread; elements content-addressed; reconcile with site → similarity → ask. Scheduler: one group at a time (V1), one driver per element, turn boundaries, What's new injected at boundaries, resume on restart from the session record. Gate runner: project test command, `arch check`, judge (fresh driver invocation, fixed prompt, structured per-element verdict), fix-round budget 2 then the four-door question; records written as files with witnesses. Resolve element for conflicts (both intents in context, fast gate). Context pack builder (map as text, rules, open findings, area descriptions, element position). Take over / hand back as attribution flips plus thread messages. Concurrency warning at plan time.

**arch-forge.** `trait Forge { pr(branch) · checks(pr) · reviewers(pr) · merge(pr, strategy) · create_pr(...) · push(branch) }`. `github` adapter (REST/GraphQL), token from the CLI's secret provider (OS keychain). UI words come from the adapter (PR · checks). GitLab later; nothing in the trait names a forge.

**arch-driver.** `trait Driver { start(task, context) · resume(session_id, message) · interrupt · stop }` returning a stream of `TurnEvent {text · tool_call · tool_result · subagent · result}`. `claude-code` adapter: builds `claude -p --bare --output-format stream-json --verbose --append-system-prompt-file … --settings {hooks} --mcp-config {arch} --allowedTools … --permission-mode acceptEdits --permission-prompts none`, in the worktree; parses stream-json; exposes MCP tools `read` (returns content + hash), `check`, `commit` (writes the Conventional Commit with trailers), `ask` (records the question, returns "wait", the turn ends); implements `arch hook pre` (stale-write guard by hash, element-scope veto, deny with reason) and `arch hook post` (attribution). Day-one test: hooks passed via `--settings` fire under `--bare`.

**arch-api.** One method set (views, intents, sessions, Ask, settings) with a JSON schema; served over JSON-RPC on a local socket for the app and over MCP for agents, same handlers. The schema is `schemas/arch-api.json`, committed and pinned by commit, with `arch serve`'s socket, framing, `initialize` and semver versioning as [ADR 0034](../adr/0034-arch-api-wire-contract.md) fixes them; `arch-app` generates its client from it.

**arch-cli.** `arch init` (areas.toml with computed sides and order, rules template, gitignore, one commit), `arch serve` (the daemon), `arch check` (CI: facts for the diff, findings, exit code, machine-readable output), `arch mcp` (the MCP server for a session), `arch hook pre|post`. Keychain access. Config (worktree path, driver, test command, me-first).

## 3. Build order and milestones

1. `arch-facts` with the store and `.arch` formats; golden tests pass on the fixtures' facts.
2. `arch-analyze` reproduces the golden facts for ripgrep and zero2prod; perf budget met; fallback path tested on a broken crate.
3. `arch-views` snapshots on fixtures; MAP invariants from `arch-fixtures` pass.
4. **`arch check` on zero2prod in CI**: the first useful binary, before any UI; findings with witnesses, exit code gates a PR.
5. `arch-driver` + `arch-session`: one session end to end from the CLI (`arch session new "…" --delegate`): plan, Accept, four elements, one gate, judge, PR created; threads and records on the branch; archive to notes on merge.
6. `arch-api` serving both transports; schema published.
7. Hardening: restart, conflicts (resolve element), concurrency warning, degrade.

## 4. Tests and CI

Golden facts and view snapshots from `arch-fixtures` (pinned); the dependency-direction check; perf budgets as benchmarks with thresholds; a driver test double that replays recorded stream-json; a real-CLI smoke test (opt-in, needs a Claude Code login) for the hook behaviour.

## 5. Sync with the other repos

- **`arch-design`**: before each milestone, read ADRs newer than the one in `README.md`'s "synced to" line; apply; file findings there (`from:arch`).
- **`arch-fixtures`**: pinned; bump the pin deliberately; a golden-fact diff is a reviewed change in that repo first, then the pin here.
- **`arch-app`**: the API schema is the contract; a schema change is announced in `arch-design` as an ADR or a note and shipped as a new schema version; never break a published method without one.
- **`sett`**: no direct dependency; when `arch-views` adds a row kind or state that the UI must render, note it in `arch-design` so sett sees it.

## 6. Definition of done for V1

On zero2prod and the small service: `arch init`, `arch check` in CI, and from the CLI a delegated session realizing four elements through one gate with a judge verdict, a PR created with Conventional Commits and trailers, review data available through the API, merge via GitHub with the archive written to notes; budgets met; fixtures green.
