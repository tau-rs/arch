# ADR 0011 · Worktrees: `<parent>/<repo>-w<n>`

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.11 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Accept creates a branch and a worktree for every session (spec §6). Their location must be predictable for the watcher, the Files view and the Terminal tab.

## Decision

Session worktrees are created at **`<parent>/<repo>-w<n>`**, recorded in the session, configurable.

## Consequences

- The scope selector reads `w1 · refund flow` (spec §4) from the same numbering.
- One watcher covers the repo and every worktree (spec §6 Sync); the recorded path is what it watches.
- Archive deletes the worktree and the branch; the session stays restorable from the notes ([ADR 0003](0003-threads-and-records.md)).
