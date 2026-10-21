use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::OrderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxKind {
    OrderPaid,
    OrderShipped,
}

impl OutboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxKind::OrderPaid => "order_paid",
            OutboxKind::OrderShipped => "order_shipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "order_paid" => Some(OutboxKind::OrderPaid),
            "order_shipped" => Some(OutboxKind::OrderShipped),
            _ => None,
        }
    }
}

/// One row of the `outbox` table: a shared table used as a queue.
/// `app::pay` and `app::ship` insert; `worker` dequeues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxMessage {
    pub id: i64,
    pub kind: OutboxKind,
    pub order: OrderId,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

#[async_trait]
pub trait Outbox: Send + Sync {
    async fn enqueue(&self, kind: OutboxKind, order: OrderId, payload: serde_json::Value) -> Result<(), OutboxError>;
    async fn dequeue(&self, limit: i64) -> Result<Vec<OutboxMessage>, OutboxError>;
    async fn ack(&self, id: i64) -> Result<(), OutboxError>;
    async fn release(&self, id: i64) -> Result<(), OutboxError>;
}

#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    #[error("storage failure: {0}")]
    Storage(String),
}
