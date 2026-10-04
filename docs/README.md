# docs

Crate-level design notes. Each links to arch-design ADRs by number (`tau-rs/arch-design/adr/`).

- [arch-facts](arch-facts.md) — the fact model and the published `schemas/facts.schema.json`
- [arch-views](arch-views.md) — placement and dependency-rule findings, what `arch check` reports
- [arch-cli and arch-api](arch-cli.md) — `arch init` and `arch check`, exit codes, `schemas/check.schema.json`; `arch session` is in arch-session
- [arch-driver](arch-driver.md) — the `Driver` trait, the claude-code command line, the stream-json parser and its fixtures
- [arch-session](arch-session.md) — Accept, the state machine, the core shaper, the scheduler, the gate, the judge, fix rounds, the planner, review and merge, `arch session …`
- [arch-forge](arch-forge.md) — the `Forge` trait, the GitHub adapter, token order, the recorded transport double
