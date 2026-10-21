# ADR 0019 · Conflicts: a resolve element on the session's plan, fast gate

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.19 (decisions of 2 Oct 2026, taken in the product chat)

## Context

When a session's branch conflicts with main or with your edit, the git flow page offers the Update dialog and a resolution record. The agent door must exist for conflict resolution (UX: conflicts agent-first with manual fallback, spec §10).

## Decision

- A conflict becomes a **resolve element** on the session's plan, with both intents in its context.
- It runs through a **fast gate**: tests, `arch check`, one closed judge question, **no fix-round loop**.
- Running sub-agents are **paused, then told**.
- The **manual door is unchanged** (the Update dialog: Rebase · Merge · Stash · Shelve · Keep).

## Consequences

- The resolution record is written either way, by the agent or by hand ([ADR 0003](0003-threads-and-records.md)).
- The element identity rule applies ([ADR 0021](0021-element-identity.md)); a resolve element is an ordinary element to review.
- The red still frame marks the collision until resolved (spec §4).
