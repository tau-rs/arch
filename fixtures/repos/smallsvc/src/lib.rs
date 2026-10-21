//! `orderly` — the `smallsvc` fixture for arch.
//!
//! A deliberately small order service laid out as a hexagon:
//!
//! ```text
//! driving            domain                     driven                 externals
//! adapters::http  →  app · domain · ports   ←   adapters::postgres  →  sql (postgres)
//!                                                adapters::memory
//!                                                adapters::stripe    →  http (api.stripe.com)
//!                                                adapters::carrier   →  http (carrier api)
//!                                                adapters::email     →  http (mail api) · tty
//! ```
//!
//! What it exercises for arch, by construction:
//! - a port with two adapters: [`ports::OrderRepository`] ← `PgOrderRepository`, `InMemoryOrderRepository`
//! - an external touched through an adapter: Stripe over HTTP, Postgres over SQL
//! - a route → handler with a middleware stack: see [`adapters::http::router`]
//! - a migration that creates tables, one of which is a shared table used as a queue (`outbox`)
//! - one rule violation on purpose, allowed in `.arch/allows`: `app::notify` names a driven adapter

pub mod adapters;
pub mod app;
pub mod config;
pub mod domain;
pub mod ports;
pub mod worker;

pub use app::Services;
