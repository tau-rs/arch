# ADR 0007 · Unit in V1: the main `[[bin]]` or the lib, and its closure

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.7 (decisions of 2 Oct 2026, taken in the product chat)

## Context

V1 has one unit per repo (spec §3). A Cargo workspace may hold several bins, examples and tool bins; the analyzer must know which one is the unit.

## Decision

- The unit is the main `[[bin]]` (or the lib) and its closure.
- Other bins and examples are "**not analyzed**" in the status line.
- `areas.toml` can name the bin.

## Consequences

- Tool-bin and example detection (spec §7) feeds the "not analyzed" state rather than the map.
- Multi-crate units and unit grouping in `.arch/units` are V2 (`roadmap.md`).
- The status-bar state is a click to its owner, never a modal ([ADR 0023](0023-settings-errors-secrets-packaging-telemetry.md)).
