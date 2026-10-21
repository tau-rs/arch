# smallsvc · `orderly`

A ~130-item order service written for `arch`'s fixtures. It exists to be analyzed, not to run in production,
but it compiles, its tests pass, and every flow is real code.

| flow | route | use case | externals touched |
|---|---|---|---|
| place | `POST /orders` | `app::PlaceOrder` | postgres (sql) |
| pay | `POST /orders/:id/pay` | `app::PayOrder` | stripe (http), postgres |
| ship | `POST /orders/:id/ship` | `app::ShipOrder` | carrier (http), postgres |
| notify | outbox worker | `app::NotifyCustomer` | mail api (http), postgres |

What arch should find, by construction:

- **a port with two adapters**: `ports::OrderRepository` ← `adapters::postgres::PgOrderRepository`, `adapters::memory::InMemoryOrderRepository` (same for `ports::Outbox`).
- **externals**: `api.stripe.com` and the carrier/mail APIs over `reqwest`; Postgres over `sqlx`.
- **route → handler with middleware**: `adapters::http::router` — `TraceLayer` → `request_id` → `require_api_key` → handler.
- **a migration with tables**: `migrations/*.sql` creates `orders`, `order_lines`, `payments`, `shipments`, `outbox`.
- **a shared table used as a queue**: `outbox` — `PgOutbox::enqueue` inserts (from `PayOrder`, `ShipOrder`), `PgOutbox::dequeue` claims with `FOR UPDATE SKIP LOCKED` (from `worker`).
- **one finding behind an allow**: `app::notify` imports `adapters::email::LogNotifier` (domain → driven); the site is listed in `.arch/allows`.

```
cargo test        # domain unit tests + the three flows on in-memory adapters
cargo run         # needs DATABASE_URL, ORDERLY_API_KEY, STRIPE_SECRET_KEY
```
