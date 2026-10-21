use async_trait::async_trait;

use crate::domain::{Money, OrderId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef(pub String);

#[async_trait]
pub trait PaymentGateway: Send + Sync {
    async fn charge(&self, order: OrderId, amount: Money) -> Result<ProviderRef, GatewayError>;
    async fn refund(&self, provider_ref: &ProviderRef) -> Result<(), GatewayError>;
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("declined: {0}")]
    Declined(String),
    #[error("provider unreachable: {0}")]
    Unreachable(String),
    #[error("unexpected provider response: {0}")]
    Protocol(String),
}
