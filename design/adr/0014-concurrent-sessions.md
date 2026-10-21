# ADR 0014 · Concurrent sessions: warn, show overlaps, never lock

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.14 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Several sessions may plan changes to the same site. Locking would block (against P-2) and would not stop a `you` session edited by hand.

## Decision

- At plan time, **warn** when a site is in another session's plan.
- **Show overlaps**.
- **Never lock.**

## Consequences

- Overlap is a map overlay fact and a planner-thread line, not a modal.
- Collisions that reach a worktree are handled by the Sync path and conflicts by a resolve element ([ADR 0019](0019-conflicts-resolve-element.md)); the red still frame shows a collision (spec §4).
