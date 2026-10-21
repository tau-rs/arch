# ADR 0003 · Threads and records: on the session's branch, archived to git notes at merge

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.3 (decisions of 2 Oct 2026, taken in the product chat)

## Context

A session produces a thread (planner then agent), a plan and records (gate outputs, judge verdicts, denials, overrides, resolution records). They must travel with the branch while it lives, survive the branch and worktree being deleted at archive ("archived keeps plan, threads, remarks, decisions", spec §10), and not depend on the driver's own transcript format.

## Decision

- While a session lives, its files are on the session's branch under `.arch/sessions/<id>/`: `thread.jsonl`, `plan.toml`, `records/`.
- At merge they move to a **git notes ref** on main (`refs/notes/arch`): that is the archive.
- The thread is **arch's own**: the filtered driver stream, plus the driver's session id and transcript path as pointers.
- Records are: gate outputs, judge verdicts, denials, overrides, resolution records.

## Consequences

- The session's branch carries `.arch/sessions/<id>/` commits; merge strips them into notes, so main's tree never holds session folders.
- **archived** is an arch fact distinct from **merged** (a forge fact), as the review flow shows (spec §6); an archived session is restorable from the notes.
- Switching drivers does not lose a thread: the driver's transcript is a pointer, not the record.
- Review of a hand-made branch has no session folder until a remark creates the first element ([ADR 0022](0022-review-without-plan.md)).
