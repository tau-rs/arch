# ADR 0020 · Plan drafts: in the cache until Accept; discard deletes

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.20 (decisions of 2 Oct 2026, taken in the product chat)

## Context

A plan is drafted and shaped before anything is created (spec §6). "Nothing is written before you say so"; discard must leave no trace.

## Decision

- A plan draft lives in the **cache** (`.arch/cache/`) until **Accept**.
- **Discard deletes** it.

## Consequences

- Accept creates the branch, the worktree and `.arch/sessions/<id>/plan.toml` with the thread (spec §6) and moves the draft out of the cache.
- Save plan writes the plan as a locked `you` session, so it is no longer a draft.
- A draft is never on a branch, so it is never in the archive.
