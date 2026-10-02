# arch · the engine

**arch** is a desktop tool for Rust codebases in which a person and coding agents work on a repository through its architecture map. This repository is the engine: one Cargo workspace, eight crates, one binary (`arch`). Everything that computes, stores, schedules or talks to agents and the forge. No UI.

The record of decisions lives in [`tau-rs/arch-design`](https://github.com/tau-rs/arch-design): the spec, the flow pages and the ADRs. This repo implements them and does not decide alone; a decision not in `adr/` is asked there as an issue labelled `from:arch`.

**Synced to: ADR 0024** (2026-10-02).

See `handoffs/handoff-arch.md` in arch-design for the engineering brief.

## Contracts published here

- `schemas/facts.schema.json` — the fact document (`facts.json`) that `arch-analyze` produces and `arch-fixtures` pins as golden files. Generated from `crates/arch-facts/src/model.rs`; `cargo test -p arch-facts` fails when the committed file drifts. Rules in `docs/arch-facts.md`.
- `fixtures/pin.toml` — the arch-fixtures commit and the target repositories (zero2prod for milestone 4).

## Findings

`FINDINGS.md` records findings that change a design decision (`from:arch`); they move to arch-design as ADRs or notes. F-1: hooks passed via `--settings` do not fire under `claude -p --bare`; the driver drops `--bare`.

## Develop

```
cargo test --workspace
scripts/check-dep-direction.sh
ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts   # regenerate schemas/facts.schema.json after a type change
```
