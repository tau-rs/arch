//! Driven adapter over Postgres (external: sql). Tables come from `migrations/`.

pub mod orders;
pub mod outbox;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub use orders::PgOrderRepository;
pub use outbox::PgOutbox;

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(8).connect(url).await
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
