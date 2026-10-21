# ADR 0015 · Restart: resume each running session at its last turn boundary

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.15 (decisions of 2 Oct 2026, taken in the product chat)

## Context

arch is a desktop app spawning the engine ([ADR 0023](0023-settings-errors-secrets-packaging-telemetry.md)); it will be closed and reopened with sessions running.

## Decision

- Scheduler state lives in the **session record**.
- On restart, each running session **resumes at its last turn boundary** with a "restarted" message in its thread.

## Consequences

- No external scheduler store; the session's records are enough to resume.
- The "restarted" message is a thread line the person and the agent both see; the agent gets its What's new at that boundary as usual (spec §6).
- Parallel groups in the scheduler stay out of V1 (spec §11).
