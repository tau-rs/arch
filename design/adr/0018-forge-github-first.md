# ADR 0018 · Forge: GitHub first, GitLab in V1.x, `Forge` trait from day one

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.18 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Spec §9 said GitLab first. Review and merge depend on a forge for the PR/MR, the checks and the merge with the project's strategy.

## Decision

- **GitHub first**: PR, checks, merge with the repo's allowed strategy.
- **GitLab in V1.x.**
- A **`Forge` trait from day one**; the UI's words (PR or MR, pipeline or checks) come from the adapter.

## Consequences

- Spec §9 ("Forge: GitLab first") is superseded by this ADR; the section is to be rewritten to reference it.
- Pages that say MR and pipeline are read as the adapter's words for that forge.
- The merge strategy, squash or merge commit, is a forge fact ([ADR 0016](0016-commits.md)); arch never merges on its own (spec §6).
