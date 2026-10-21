# arch · the engine

**arch** is a desktop tool for Rust codebases in which a person and coding agents work on a repository through its architecture map. This repository holds the engine and its record:

- `crates/` — one Cargo workspace, eight crates, one binary (`arch`). Everything that computes, stores, schedules or talks to agents and the forge. No UI.
- `design/` — the record of decisions: the spec, the flow pages, the ADRs and the handoffs (`design/README.md`). The engine implements them and does not decide alone; a decision not in `design/adr/` is filed as an issue labelled `from:arch` and answered with an ADR before code.
- `fixtures/` — the ground truth: real repos pinned as submodules plus the in-tree smallsvc, the golden facts, and the invariant harness (`fixtures/README.md`).

Until 2026-10-21 `design/` and `fixtures/` were the repos `tau-rs/arch-design` and `tau-rs/arch-fixtures`; both are archived and their histories are merged here (#100).

See `design/handoffs/handoff-arch.md` for the engineering brief.

## Use

```
arch init           # writes .arch/areas.toml and .arch/rules, one commit, no questions
arch check          # findings against .arch/rules; exit 0 clean or warnings, 1 blocking, 2 tool error
arch check --format json
```

Details in `docs/arch-cli.md`. CI runs both on zero2prod at the commit in `fixtures/pin.toml` and fails on a blocking finding or a tool error (`scripts/check-zero2prod.sh`, milestone 4).

## Contracts published here

- `schemas/check.schema.json` — what `arch check --format json` prints. Generated from `arch_api::CheckOutput`; drift-tested.

- `schemas/facts.schema.json` — the fact document (`facts.json`) that `arch-analyze` produces and `fixtures/golden/` pins. Generated from `crates/arch-facts/src/model.rs`; `cargo test -p arch-facts` fails when the committed file drifts. Rules in `docs/arch-facts.md`.
- `fixtures/pin.toml` — the target repositories `scripts/check-zero2prod.sh` clones (zero2prod for milestone 4).

## Findings

`FINDINGS.md` records findings that change a design decision (`from:arch`); they become ADRs under `design/adr/` or notes. F-1: hooks passed via `--settings` do not fire under `claude -p --bare`; the driver drops `--bare`.

## Develop

```
cargo test --workspace
scripts/check-dep-direction.sh
ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts   # regenerate schemas/facts.schema.json after a type change
scripts/check-zero2prod.sh                      # milestone 4 proof: arch init + arch check on zero2prod at the pin
```
