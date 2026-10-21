# ADR 0006 · `arch init`: one commit, no questions

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.6 (decisions of 2 Oct 2026, taken in the product chat)

## Context

The onboarding funnel is out of V1 (spec §11). A repo without `.arch/` must be usable on day one (spec §12).

## Decision

`arch init` writes:
- `areas.toml` with computed sides and order (the sides rule: [ADR 0027](0027-arch-init-sides-rule.md); the order rule and what is written per area: [ADR 0029](0029-arch-init-order-rule.md));
- `rules` from the template: domain must not depend on driving or driven; externals only from driven; the five lints on (the file format and the lint names: [ADR 0032](0032-rules-allows-format-five-lints.md));
- the gitignore line for `.arch/cache/`;

in **one commit**, asking **no questions**.

## Consequences

- The CLI is `arch init · serve · check · mcp · hook` ([ADR 0023](0023-settings-errors-secrets-packaging-telemetry.md)); `init` is the V1 onboarding.
- The template rules are the first findings a repo sees; `allows` are added by hand, person-only (spec §7).
- A team that disagrees with a computed side edits `areas.toml` ([ADR 0004](0004-areas-derived-no-source-annotation.md)); nothing else is asked.
