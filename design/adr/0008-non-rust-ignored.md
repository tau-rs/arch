# ADR 0008 · Non-Rust: ignored, except table names from `migrations/*.sql`

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.8 (decisions of 2 Oct 2026, taken in the product chat)

## Context

arch is for Rust codebases (spec §1). Repos also hold SQL, config, scripts and front-ends. The fact model needs `shared table used as a queue` and data-store ports (spec §5, §7), which need table names.

## Decision

Non-Rust files are **ignored**, with one exception: **table names** are read from `migrations/*.sql`.

## Consequences

- The sql port kind and the "who inserts, who dequeues" fact can be resolved against real table names.
- Anything else non-Rust is neither an item nor a witness in V1; cross-language facts are a plugin or V3 concern.
