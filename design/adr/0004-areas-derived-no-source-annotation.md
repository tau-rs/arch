# ADR 0004 · Areas: derived from the module tree, overrides only, no annotation in source

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.4 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Spec §1 described one `//! @arch area:` line per file as the single thing arch writes into source. P-2 (reader, not owner) and "nothing is written before you say so" argue for writing nothing into source at all. Areas can be derived from the module tree for the common case.

## Decision

- Areas are **derived from the module tree**.
- `.arch/areas.toml` holds **only overrides**: path patterns → area, side, order, main bin.
- Optional `.arch/areas/<name>.md` holds a description per area, read into the context pack.
- **No annotation in source**: the `//! @arch area:` line is dropped. Keep writes `.arch/areas.toml` only.

## Consequences

- Spec §1 ("writes almost nothing into source (one `//! @arch area:` line per file)") is superseded by this ADR; the section is to be rewritten to reference it.
- Editing areas on the Map (drag, rename, draw) produces one change to `areas.toml` on Keep, undoable until then (spec §5).
- A new item without an override is placed by derivation; the Unplaced tray (spec §6 Sync) holds what derivation cannot place.
- `arch init` computes sides and order into `areas.toml` ([ADR 0006](0006-arch-init.md)).
