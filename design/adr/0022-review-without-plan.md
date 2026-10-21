# ADR 0022 · Review without a plan: works for any branch

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.22 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Branches made by hand, or pushed by someone without arch, still need to be reviewed and merged in arch. P-1 requires the manual path to be always present.

## Decision

- Review **works for any branch**; the scope line reads **`plan · none · hand-made branch`**.
- A remark that asks for a change **creates the first element**, and with it the session's plan.

## Consequences

- The checklist line "plan realized" is absent or n/a for such a branch; the other lines (findings, checks, remarks, current with main, files viewed) apply.
- A `you` session's "Delegate the rest" (spec §4) and this path are the two ways a plan appears after the fact.
