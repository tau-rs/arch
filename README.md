# arch · the engine

**arch** is a desktop tool for Rust codebases in which a person and coding agents work on a repository through its architecture map. This repository is the engine: one Cargo workspace, eight crates, one binary (`arch`). Everything that computes, stores, schedules or talks to agents and the forge. No UI.

The record of decisions lives in [`tau-rs/arch-design`](https://github.com/tau-rs/arch-design): the spec, the flow pages and the ADRs. This repo implements them and does not decide alone; a decision not in `adr/` is asked there as an issue labelled `from:arch`.

**Synced to: ADR 0024** (2026-10-02).

See `handoffs/handoff-arch.md` in arch-design for the engineering brief.
