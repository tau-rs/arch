# ADR 0023 · Settings, errors, secrets, packaging, telemetry

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.23 (decisions of 2 Oct 2026, taken in the product chat)
- Settled since: tau-rs/arch-design#15 is decided (2026-10-03): the driver uses the user's normal Claude Code login and arch requires no API key; a consequence says so
- Settled since: tau-rs/arch-design#45 is decided (2026-10-21): the engine binary stays `arch`; a program trusts a candidate only if `--version` prints `arch <semver>` first, and the packaged app runs its bundled engine before looking on `PATH` ([ADR 0036](0036-engine-binary-name.md))

## Context

Cross-cutting decisions the shell and the engine need before the first screen: where settings live, how errors surface under P-2, where secrets go, how the app and engine ship, and whether anything is reported home.

## Decision

- **Settings**: a Settings tab (this repo · you · appearance).
- **Errors**: a status-bar state per subsystem plus a Checks row; **never a modal**; the last good view is kept.
- **Secrets**: the OS keychain via Theia; **never in `.arch`**.
- **Packaging**: a Theia desktop app spawning **`arch`** as a separate binary; the `arch` CLI is **`init · serve · check · mcp · hook`**; the binary keeps the name `arch` next to the system `arch`, told apart by `arch --version` ([ADR 0036](0036-engine-binary-name.md)).
- **Telemetry**: **none**.

## Consequences

- `arch-app` and `arch` are separate repos and processes ([ADR 0024](0024-repositories.md)); the API schema is their contract.
- Degraded analysis ([ADR 0010](0010-degrade.md)) and forge failures surface through the same status-bar and Checks path.
- `arch check` is the CI entry; `arch mcp` serves agents; `arch hook` is what the driver's hooks call ([ADR 0012](0012-confinement-driver-hooks.md)).
- The driver runs under the user's normal Claude Code login, the same one `claude` uses in their terminal; no API key is required ([ADR 0012](0012-confinement-driver-hooks.md), tau-rs/arch-design#15). Why: `--bare` would have forced per-call API billing on every user and locked out claude.ai logins.
