# ADR 0021 · Element identity: content-addressed, `E3` as label, continuity by site then similarity

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.21 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Elements are referenced from commits (trailer), from the map (amber overlay), from review (remark → element), and across reconciles when a plan is rewritten (spec §6 Sync). They need a stable id that survives edits to the plan without anything in source.

## Decision

- An element's id is **`sha256(session · intention · site)[:8]`**; **`E3`** is its display label.
- Continuity across rewrites is **by site, then by similarity, then by a question in the thread**.
- **`plan.toml` owns the element**; nothing in source.

## Consequences

- Gutter marks and hints in the editor come from `plan.toml`, with no inserted lines (spec §6).
- Reconcile's impact analysis (element rewritten ≠, dependent elements stale) is computed on these ids.
- Commit trailers `Arch-Element` ([ADR 0016](0016-commits.md)) carry the id, not the label.
