# ADR 0016 · Commits: agents commit through arch's tool, one per element, with trailers

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.16 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Commits are facts (spec §3, §7): the plan element a commit belongs to must be known to the map, the review and the archive. Agents must not write history arch cannot attribute.

## Decision

- Agents commit **through arch's tool**: Conventional Commits `type(area): summary`, body from the intention, trailers **`Arch-Element`**, **`Arch-Session`**, **`Co-authored-by`**; **one commit per element**.
- Direct `git commit` is **denied to agents**.
- **Your commits are free**; the trailer is prefilled when attached to an element.
- The merge strategy comes from the forge.

## Consequences

- The Changes list and the review checklist map commits to elements by trailer; a hunk returns to unviewed on commit (spec §6).
- The commit-from-the-chip path (spec §4, §6 Daily) prefills `Arch-Element:` from the diff or the plan element.
- Squash vs merge commit is read from the forge ([ADR 0018](0018-forge-github-first.md)), never chosen in arch.
