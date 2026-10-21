# ADR 0009 · Confidence: `resolved` · `guessed` · `declared`

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.9 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Links are resolved or unresolved with a reason (spec §7); dyn and spawn paths are folded (spec §5); a degraded crate yields syntax-level facts ([ADR 0010](0010-degrade.md)); a team may declare an edge by hand. A finding must not block a merge on a fact arch is not sure of.

## Decision

- Every fact carries a confidence: **`resolved`** · **`guessed`** · **`declared`**.
- A finding on a **guessed** link **warns, never blocks**.
- A finding on a **declared** link **cannot gate a merge**.

## Consequences

- The gate ([ADR 0013](0013-judge.md), spec §6) and the merge checklist count only findings on resolved links as blocking.
- The map paints confidence (lightest ink for guessed, spec §13.10); the witness rule still applies to every level.
- The `origin` field on findings stays `core` in V1; confidence is a separate axis.
