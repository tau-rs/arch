# ADR 0024 · Repositories: arch · arch-app · sett · arch-fixtures · arch-design

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.24 (decisions of 2 Oct 2026, taken in the product chat)

## Context

The engine, the Theia product, the design system, the test data and the record have different languages, cadences and consumers; one repo would couple their builds and their histories.

## Decision

Five repositories under `tau-rs`:
- **`arch`**: the engine, one Cargo workspace.
- **`arch-app`**: the Theia product.
- **`sett`**: the design system.
- **`arch-fixtures`**: pinned repos, golden facts, invariant checks.
- **`arch-design`**: spec, pages, ADRs (this repo).

See the per-repo handoffs under `handoffs/`.

## Consequences

- No repo depends on `arch-design` at build time; each states in its README the ADR number it is synced to (`handoffs/handoff-arch-design.md`).
- Findings from a repo are filed here as issues labelled `from:<repo>`; the product chat triages them into ADRs.
- A decision not in `adr/` is asked of the product chat; no repo decides alone.
