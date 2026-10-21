# V1 done · the end-to-end scenario

"V1 is done" means this scenario runs on `smallsvc` and on `zero2prod` and every expected row appears. Rows marked
**by construction** are fixed by the fixture's source and must match exactly; rows marked **from golden** are
confirmed when `arch-analyze` produces `golden/<repo>/facts.json`, and the number here is the expected one.

Steps: init · check · session · gate · judge · PR · review · merge · archive. Each step lists the command or the
UI action, then the rows the status bar, the Findings tab, the Sessions view or the checklist must show.

---

## A · smallsvc (`repos/smallsvc`, crate `orderly`)

### 1 · init

`arch init` (ADR 6): derive areas from the module tree, write `.arch/areas.toml` overrides, the rules template, the
gitignore line, one commit, no questions. smallsvc ships its `.arch/` already, so `arch init` is a no-op here; the
rows below are what the map shows after the first index.

| row | expected | source |
|---|---|---|
| unit | `orderly` · main bin `orderly` · hexagon | by construction (`areas.toml`) |
| areas · driving | `http` | by construction |
| areas · domain | `app` · `domain` · `ports` | by construction |
| areas · driven | `postgres` · `memory` · `stripe` · `carrier` · `email` | by construction |
| externals · data stores | `postgres` (sql) via `sqlx` — tables `orders` `order_lines` `payments` `shipments` `outbox` | by construction (`migrations/`) |
| externals · third-party | `api.stripe.com` (http) · carrier api (http) · mail api (http) | by construction |
| entries | `main` (framework-held: axum) · outbox worker loop (`worker::run_outbox_worker`) | by construction |
| routes → handlers | 5: `GET /health` · `POST /orders` · `GET /orders/:id` · `POST /orders/:id/pay` · `POST /orders/:id/ship`; middleware stack `TraceLayer → request_id → require_api_key` (health skips the key) | by construction (`adapters/http/mod.rs`) |
| ports | `OrderRepository` (2 adapters) · `Outbox` (2) · `Notifier` (2) · `PaymentGateway` (1) · `ShippingProvider` (1) | by construction |
| shared table used as a queue | `outbox`: inserts `PgOutbox::enqueue` ← `PayOrder::run`, `ShipOrder::run`; dequeue `PgOutbox::dequeue` ← `worker` | by construction |
| facades | `domain/mod.rs`, `ports/mod.rs` (only `pub use`) | from golden |
| unplaced tray | empty | by construction |
| status bar | `1 unit · 9 areas · N items · 0 unplaced` | N from golden (`sizes.json` after analyze) |

### 2 · check

`arch check` on main.

| row | expected | source |
|---|---|---|
| finding | `domain must not depend-on driven` · site `src/app/notify.rs::NotifyCustomer::deliver` → `adapters::email::LogNotifier` · confidence **resolved** · **allowed** by titouan ("log fallback while the mail provider is unreliable…") | by construction (`.arch/allows`) |
| blocking | 0 | by construction |
| exit code | 0 | by construction |
| direction smells | see FINDINGS.md F-5: driven adapters reference domain types; the expected count depends on the decision | gated |
| degraded crates | none (`cargo check` clean) | by construction |

### 3 · session

Plan: **cancel a paid order** — refund through the gateway, mark cancelled, notify the customer.
Door used: `+ new session · delegate`. Scope selector reads `plan · cancel order` (amber).

| row | expected |
|---|---|
| elements (amber on the map) | `E1` `app::cancel::CancelOrder` (domain, new file `src/app/cancel.rs`) · `E2` `OutboxKind::OrderCancelled` + `Notification::order_cancelled` (domain) · `E3` route `POST /orders/:id/cancel` → `handlers::cancel_order` (driving) |
| element ids | `sha256(session · intention · site)[:8]`, labels `E1 E2 E3` (ADR 21) |
| groups (core shaper, by dependency) | `G1 = {E2, E1}` · `G2 = {E3}` |
| gates | one per group: `arch check` + `cargo test` (spec §8) |
| warnings at plan time | none (no other session claims these sites; ADR 14) |
| Accept · delegate | branch `arch/cancel-order`, worktree `<parent>/smallsvc-w1`, `.arch/sessions/<id>/plan.toml` + `thread.jsonl`; row moves Planning → Running; frame violet |
| Sessions view | `w1 · cancel order` › `G1` › sub-agent on `E2` (running) · `E1` (queued) › `G2` › `E3` (queued) |
| Files view | a 3 px bar in the session colour on `src/domain/notification.rs`, `src/ports/outbox.rs` while written |
| deviation (expected once, by design of the fixture) | the `E1` sub-agent tries to edit `src/adapters/email/mod.rs` to add the cancelled template to `LogNotifier`; the core veto denies the write (outside `E1`'s element files), the thread shows the reason and the three typologies |
| stale-write guard | not triggered unless you edit a file the agent holds |

### 4 · gate

`G1` ends.

| Checks tab row | expected |
|---|---|
| `cargo test` | ok · 6 existing + the new tests, output as witness |
| `arch check` | 0 blocking · 1 allowed (unchanged) · 0 new |
| fix rounds used | 0 of 2 |

### 5 · judge

Same provider, fresh invocation, per-element verdict (ADR 13).

| row | expected |
|---|---|
| `E2` | realized · reason cites `OutboxKind::OrderCancelled` and the template |
| `E1` | realized · reason cites `CancelOrder::run` calling `PaymentGateway::refund` then `Order::cancel` |
| `E3` (after `G2`) | realized · reason cites the route and the handler |
| verdict overridable | yes, with a recorded reason under `records/` |

### 6 · PR

`create MR` from the Done card (ADR 17: push happens here, never by the agent).

| row | expected |
|---|---|
| commits | 3, one per element, `feat(app): cancel a paid order` … with trailers `Arch-Element: E1`, `Arch-Session: <id>`, `Co-authored-by` (ADR 16) |
| description | drafted from the plan and the thread |
| forge | GitHub PR (ADR 18); pipeline rows appear on the Checks tab, read never run |

### 7 · review

| checklist row | expected |
|---|---|
| plan realized | ✓ 3 of 3 |
| gates and judge | ✓ |
| findings | ✓ 0 new (1 allowed, pre-existing) |
| pipeline | ✓ when green |
| remarks | one `comment · no change needed` on `src/app/cancel.rs` ("refund before cancel: fine, but say why in a doc comment") — resolution in the finding's row |
| current with main | ✓ |
| files viewed | optional |
| verdict | approve |

### 8 · merge

| row | expected |
|---|---|
| strategy | from the forge's default (squash on this repo) |
| button | `merge · squash · delete branch · archive session` — says what it does |
| result pills | **merged** (forge fact) · **archived** (arch fact) |

### 9 · archive

| row | expected |
|---|---|
| branch, worktree | gone |
| `.arch/sessions/<id>/` | moved to `refs/notes/arch` on main (ADR 3); plan, thread, remarks, verdicts restorable |
| Sessions view | row under Done |
| What's new | `map delta: +3 items, +1 route` · `findings ±0` |

---

## B · zero2prod (`repos/zero2prod`, pin `970987c`)

### 1 · init

`arch init` writes `.arch/` into the worktree (not into the pinned submodule; the scenario runs on a copy).

| row | expected | source |
|---|---|---|
| unit | `zero2prod` · main bin · hexagon (an entry exists) | from golden |
| areas (derived from modules) | `routes` (driving) · `domain` · `authentication` · `idempotency` · `email_client` (driven) · `issue_delivery_worker` · `startup` `configuration` `telemetry` `session_state` `utils` (app) | side of `authentication`, `idempotency`, `issue_delivery_worker` is a guess; see FINDINGS.md F-6 |
| externals · data stores | `postgres` (sql): tables `subscriptions` `subscription_tokens` `users` `idempotency` `newsletter_issues` `issue_delivery_queue` · `redis` (session store) | by construction (`migrations/`, `startup.rs`) |
| externals · third-party | Postmark (http) via `email_client` | by construction |
| entries | `main` (framework-held: actix-web) · `issue_delivery_worker::run_worker_until_stopped` | by construction |
| routes → handlers | 13: `GET /` · `GET,POST /login` · `GET /health_check` · `POST /subscriptions` · `GET /subscriptions/confirm` · `POST /newsletters` · under `/admin` with `reject_anonymous_users`: `GET /dashboard` · `GET,POST /newsletters` · `GET,POST /password` · `POST /logout`; middleware `TracingLogger → SessionMiddleware → FlashMessagesFramework` | by construction (`startup.rs`) |
| shared table used as a queue | `issue_delivery_queue`: insert `routes/admin/newsletter/post.rs` (`enqueue_delivery_tasks`); dequeue + delete `issue_delivery_worker.rs` (`dequeue_task`, `delete_task`) | by construction |
| ports | none declared as traits — the repo talks to externals directly | by construction |

### 2 · check

With the template rules (ADR 6) and no allows:

| row | expected | source |
|---|---|---|
| `driving must not depend-on externals` | 6 sites, one per route file that holds a `PgPool`/`sqlx` call: `routes/subscriptions.rs` · `routes/subscriptions_confirm.rs` · `routes/login/post.rs` · `routes/admin/dashboard.rs` · `routes/admin/newsletter/post.rs` · `routes/admin/password/post.rs`; plus `routes/subscriptions.rs` → `reqwest` | by construction (file list); link count from golden |
| `domain must not depend-on …` | 0 (the three domain types import nothing) | by construction |
| blocking | the 7 above, if the template's level is `block` | from golden |
| exit code | 1 | from golden |

### 3–9 · session → archive

Plan: **put subscriptions behind a port** — `SubscriptionRepository` trait + `PgSubscriptionRepository`,
`routes/subscriptions.rs` calls the port. Elements `E1` port (domain) · `E2` adapter (driven) · `E3` handler change
(driving). Groups `{E1} → {E2} → {E3}`.

| step | expected |
|---|---|
| gate `G1` | `cargo test` ok (the repo's integration tests need Postgres and Redis; the gate records them as skipped with the reason if absent) · `arch check` unchanged |
| gate `G3` | `arch check` reports **one fewer** `driving → externals` finding (6 sites → 5) |
| gate failed (forced) | re-run `G3` with `ORDERLY_FORCE_FAIL=1`-style injection is not available on zero2prod; instead the scenario asks the sub-agent for a wrong port signature once; the fix-round budget (2) is spent on round 1, the session continues |
| judge | per-element verdicts; `E3` reason cites the handler now calling `SubscriptionRepository::insert` |
| PR · review · merge · archive | as in A, with the checklist's findings row reading `✓ −1` |

---

## What this scenario does not cover (out of V1 or elsewhere)

Onboarding; plugins; units and board placement; multi-repo; the performance budgets (checks/budgets.toml, separate issue);
take over and hand back (asserted in the arch-app flow tests, not here).
