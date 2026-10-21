use async_trait::async_trait;

use crate::domain::{Order, OrderId, OrderStatus, Payment, Shipment};

/// The port with two adapters in this repo: `adapters::postgres::PgOrderRepository`
/// and `adapters::memory::InMemoryOrderRepository`.
#[async_trait]
pub trait OrderRepository: Send + Sync {
    async fn get(&self, id: OrderId) -> Result<Order, RepoError>;
    async fn save(&self, order: &Order) -> Result<(), RepoError>;
    async fn list_by_status(&self, status: OrderStatus) -> Result<Vec<Order>, RepoError>;
    async fn record_payment(&self, payment: &Payment) -> Result<(), RepoError>;
    async fn record_shipment(&self, shipment: &Shipment) -> Result<(), RepoError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("order {0} not found")]
    NotFound(OrderId),
    #[error("storage failure: {0}")]
    Storage(String),
    #[error("stored row is not a valid order: {0}")]
    Corrupt(String),
}
