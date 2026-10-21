use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::OrderId;
use crate::ports::{Outbox, OutboxError, OutboxKind, OutboxMessage};

/// The `outbox` table used as a queue: `enqueue` inserts, `dequeue` claims with `SKIP LOCKED`.
pub struct PgOutbox {
    pool: PgPool,
}

impl PgOutbox {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct OutboxRow {
    id: i64,
    kind: String,
    order_id: Uuid,
    payload: serde_json::Value,
    attempts: i32,
}

fn storage(e: sqlx::Error) -> OutboxError {
    OutboxError::Storage(e.to_string())
}

#[async_trait]
impl Outbox for PgOutbox {
    async fn enqueue(&self, kind: OutboxKind, order: OrderId, payload: serde_json::Value) -> Result<(), OutboxError> {
        sqlx::query("INSERT INTO outbox (kind, order_id, payload) VALUES ($1, $2, $3)")
            .bind(kind.as_str())
            .bind(order.0)
            .bind(payload)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn dequeue(&self, limit: i64) -> Result<Vec<OutboxMessage>, OutboxError> {
        let rows: Vec<OutboxRow> = sqlx::query_as(
            "UPDATE outbox SET locked_at = now(), attempts = attempts + 1
             WHERE id IN (SELECT id FROM outbox WHERE locked_at IS NULL ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED)
             RETURNING id, kind, order_id, payload, attempts",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(|r| {
                let kind = OutboxKind::parse(&r.kind).ok_or_else(|| OutboxError::Storage(format!("unknown kind {}", r.kind)))?;
                Ok(OutboxMessage { id: r.id, kind, order: OrderId(r.order_id), payload: r.payload, attempts: r.attempts })
            })
            .collect()
    }

    async fn ack(&self, id: i64) -> Result<(), OutboxError> {
        sqlx::query("DELETE FROM outbox WHERE id = $1").bind(id).execute(&self.pool).await.map_err(storage)?;
        Ok(())
    }

    async fn release(&self, id: i64) -> Result<(), OutboxError> {
        sqlx::query("UPDATE outbox SET locked_at = NULL WHERE id = $1").bind(id).execute(&self.pool).await.map_err(storage)?;
        Ok(())
    }
}
