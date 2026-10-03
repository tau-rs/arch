# Context pack · smallsvc

Unit `unit:smallsvc`, built from `bin:orderly`, at `42c5699f836c83b4f68090c33586910ffc8c21db`.
Facts are type-checked.

## Map

### Areas

Column rule: hexagon (driving → domain → driven → externals).

- **http** · driving · src/adapters/http/** · 35 items
- **app** · domain · src/app/**, src/worker.rs, src/config.rs · 43 items
- **domain** · domain · src/domain/** · 78 items
- **ports** · domain · src/ports/** · 34 items
- **postgres** · driven · src/adapters/postgres/**, migrations/** · 28 items
- **memory** · driven · src/adapters/memory/** · 20 items
- **stripe** · driven · src/adapters/stripe/** · 13 items
- **carrier** · driven · src/adapters/carrier/** · 7 items
- **email** · driven · src/adapters/email/** · 9 items
- outside every area (no rule applies): `src/adapters/mod.rs`, `src/lib.rs`, `src/main.rs`

### Ports

- driving · `port:http:GET /health` · unresolved
- driving · `port:http:GET /orders/:id` · unresolved
- driving · `port:http:POST /orders` · unresolved
- driving · `port:http:POST /orders/:id/pay` · unresolved
- driving · `port:http:POST /orders/:id/ship` · unresolved
- driven · `port:http:carrier` · third-party · behind `external:http:carrier`
- driven · `port:http:email` · third-party · behind `external:http:email`
- driven · `port:http:stripe` · third-party · behind `external:http:stripe`
- driven · `port:sql:postgres` · data-stores · behind `external:sql:postgres`

### Entries

- framework (axum) · `orderly::adapters::http::handlers::get_order#fn` · in `http`
- framework (axum) · `orderly::adapters::http::handlers::health#fn` · in `http`
- framework (axum) · `orderly::adapters::http::handlers::pay_order#fn` · in `http`
- framework (axum) · `orderly::adapters::http::handlers::place_order#fn` · in `http`
- framework (axum) · `orderly::adapters::http::handlers::ship_order#fn` · in `http`
- spawned worker · `orderly::worker::run_outbox_worker#fn` · in `app`
- main · `orderly[bin:orderly]::main#fn` · outside every area

### Links between areas

- app → domain: calls 12, uses-type 7, holds 6, constructs 2, matches-on 1, reads 7
- app → email: calls 1, constructs 1
- app → ports: calls-port 16, depends-on-port 26, uses-type 1, holds 5, constructs 3, matches-on 3, reads 6
- carrier → domain: calls 1, uses-type 4, holds 1, constructs 1
- carrier → externals: calls-out 1
- carrier → ports: implements 1, uses-type 1, constructs 2
- email → domain: uses-type 2, constructs 1, reads 6
- email → externals: calls-out 1
- email → ports: implements 2, uses-type 2, constructs 2
- http → app: calls 4, uses-type 3, holds 1, constructs 2, matches-on 7, reads 4
- http → domain: calls 6, uses-type 8, holds 3, constructs 1, reads 14
- http → ports: calls-port 1
- memory → domain: uses-type 10, holds 4, reads 2
- memory → ports: implements 2, uses-type 11, holds 1, constructs 2, reads 3
- ports → domain: uses-type 15, holds 2
- postgres → domain: calls 7, uses-type 10, constructs 7, reads 32
- postgres → externals: reads 11, queues 2
- postgres → ports: calls 2, implements 2, uses-type 15, constructs 7
- stripe → domain: calls 1, uses-type 2, reads 2
- stripe → externals: calls-out 2
- stripe → ports: implements 1, uses-type 4, constructs 6, reads 1

## Rules

A subject or a target is a side or an area name; `externals` is everything outside the unit except libraries.

- domain must not depend-on driving, driven · block
- driving must not depend-on externals · block
- domain must not depend-on externals · block

## Open findings

None.

2 more covered by `.arch/allows`, not listed.

## Area descriptions

None written (`.arch/areas/<name>.md`).

## Your element: E3 · `8dc8c8ce`

Intention: the RefundOrder use case

- site: `app` · area `app` (domain)
- group: flow (with E4)
- after: E1 `c0736cab` · add a Refunded status and its transition; E2 `8aa4b1c4` · add refund to the payment gateway port

### Files you may write

- `src/app/refund.rs` · area `app` (domain)
- `src/app/mod.rs` · area `app` (domain)

### What you may depend on

From `app` (domain):
- may depend on: domain, ports
- must not depend on: http, postgres, memory, stripe, carrier, email (`domain must not depend-on driving, driven`); externals (`domain must not depend-on externals`)
