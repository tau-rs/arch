# ADR 0026 · Budgets: first index < 5 s cold at 1,200 items; memory < 500 MB RSS at 5,000 visible items

- Date: 2026-10-03
- Status: accepted
- Source: arch-fixtures FINDINGS.md F-7 (tau-rs/arch-design#11); completes spec §5 "performance budgets owned by arch"

## In plain words

A budget is a number the product promises to stay under, checked by one benchmark that fails the build when it is crossed. The spec names six budgets but gives numbers for only four; the two missing ones are how long the very first analysis of a repository may take, and how much memory the engine may use. This ADR gives those two numbers, says exactly what each one measures so the benchmark cannot be argued with, and names one benchmark per budget so fixtures can write `budgets.toml`. The four existing numbers do not move. One analogy: the four existing budgets are the speed limits once you are on the road; this ADR adds the limit on how long the engine may take to start and how much fuel it may carry.

## Context

- Spec §5: "performance budgets owned by arch: first paint of a 1,200-item repo < 2 s, fold/expand < 100 ms, recompute of one file < 500 ms, 60 fps pan/zoom at 5,000 visible items".
- handoff-arch-fixtures.md: `budgets.toml` lists `first index · one-file recompute · fold/expand · first paint · pan/zoom fps · memory`; "every budget has one benchmark".
- handoff-arch.md carries "first index of zero2prod < 10 s", a per-fixture figure, not a budget; tau-rs/arch#3 copied it as a checklist item.
- Sizes today (`golden/*/sizes.json`, syntactic declarations, items not yet filled): smallsvc 237, zero2prod 183, ripgrep 3,306, zed 71,418 across 253 crates. The 1,200-item line of spec §5 sits between zero2prod and ripgrep.

## Decision

| budget | number | what is measured | benchmark | runs in |
|---|---|---|---|---|
| first index | **< 5 s** | cold: no `.arch/cache/`; `cargo metadata` answers and a prior `cargo check` has left build-script and proc-macro artifacts in `target/`; from `arch serve` (or `arch init`) start to the first complete facts set for the unit at `resolved` confidence (dependencies are name-resolved, not analysed); on a repo of 1,200 items | `first_index_cold` | `arch` |
| one-file recompute | < 500 ms | unchanged (spec §5) | `recompute_one_file` | `arch` |
| memory | **< 500 MB RSS** | peak resident set of the **`arch` process alone** from start through first index and while serving a view of 5,000 visible items; the Theia renderer is outside this budget | `memory_rss_peak` | `arch` |
| fold/expand | < 100 ms | unchanged; keypress to paint, end to end | `fold_expand` | `arch-app` |
| first paint | < 2 s at 1,200 items | unchanged; engine running, app cold | `first_paint` | `arch-app` |
| pan/zoom | 60 fps at 5,000 visible items | unchanged | `pan_zoom_fps` | `arch-app` |

- **Which fixture.** A budget is a ceiling at its stated size; a smaller repo must meet it too. `first_index_cold` runs on every pin; the 1,200-item line is read against the fixture whose measured `items` is nearest 1,200 once arch-analyze fills `sizes.json` (expected: ripgrep's main unit). `memory_rss_peak` at 5,000 visible items runs on the zed pin, the only fixture above 5,000 items (V1 asserts facts for three lib roots only; the whole workspace exists for perf runs, per the fixtures handoff).
- **Reference machine.** Named once in `budgets.toml` by fixtures (arch-fixtures #5); the numbers hold there, and a developer laptop is faster, never slower.
- **Supersedes** handoff-arch.md's "first index of zero2prod < 10 s": zero2prod runs the same `first_index_cold` benchmark under the 5 s ceiling.

## Consequences

- fixtures writes `checks/budgets.toml` with these six rows and names the reference machine; `arch` CI runs the three engine benchmarks against the fixtures pin, `arch-app` CI runs the three renderer benchmarks against `fixtures-for-ui/`.
- `arch-analyze`'s plan must fit first index: analyse the unit's items only, keep dependencies at name resolution, and reuse `target/` artifacts for build scripts and proc-macros. If rust-analyzer's load alone exceeds the ceiling on the reference machine, the fallback is not a bigger number but ADR 0010's path: syntax-level facts first, marked guessed, refined in place.
- One honest consequence: 500 MB is the engine's budget, not the product's. The desktop app's total footprint is dominated by Theia and Electron and has no budget in this ADR; arch-app may set its own under `from:app`.
- A second honest consequence: "cold" still assumes a built `target/`; the true first run on a fresh clone pays `cargo check` as well, and that time is cargo's, not arch's, and is not budgeted.
