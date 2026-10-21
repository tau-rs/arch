# ADR 0013 · Judge: same provider, fresh invocation, fixed prompt, structured verdict

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.13 (decisions of 2 Oct 2026, taken in the product chat)

## Context

A gate ends with a judge (spec §6); it must be independent of the sub-agent it judges, must not become a second implementer, and must be overridable by a person with a record.

## Decision

- The judge is the **same provider**, in a **fresh invocation**, with a **fixed prompt**.
- It returns a **structured per-element verdict with a reason**.
- It **may not propose code**.
- Its verdict is **overridable with a recorded reason**.
- A second provider as judge is **V2**.

## Consequences

- Verdicts and overrides are records under `.arch/sessions/<id>/records/` ([ADR 0003](0003-threads-and-records.md)) and rows on the Checks tab.
- The judge reads the same context pack as the sub-agent ([ADR 0005](0005-context-pack.md)).
- The merge checklist's "gates and judge" line is ✓ from verdicts or recorded overrides; "accept as is with a recorded override" after a spent fix-round budget is the same mechanism (spec §6).
