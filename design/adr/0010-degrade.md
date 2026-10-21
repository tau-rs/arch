# ADR 0010 · Degrade: syntax-level facts marked guessed when rust-analyzer cannot type-check

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.10 (decisions of 2 Oct 2026, taken in the product chat)

## Context

rust-analyzer may fail to type-check a crate (broken build, missing toolchain, proc-macro failure). The map must still show something, and P-2 says nothing blocks and nothing pops up.

## Decision

A crate rust-analyzer cannot type-check **falls back to syntax-level facts marked `guessed`**. The degraded state is shown: lightest ink on the map, a status-bar state, a What's new line, and the reason on the Checks tab.

## Consequences

- Findings in a degraded crate warn and never block ([ADR 0009](0009-confidence-levels.md)).
- Errors are a status-bar state plus a Checks row, never a modal, and the last good view is kept ([ADR 0023](0023-settings-errors-secrets-packaging-telemetry.md)).
- A running session is told the degradation in its What's new at the next turn boundary.
