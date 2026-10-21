use async_trait::async_trait;

use crate::domain::{Address, Carrier, OrderId, TrackingNumber};

#[async_trait]
pub trait ShippingProvider: Send + Sync {
    async fn book(&self, order: OrderId, carrier: Carrier, to: &Address) -> Result<TrackingNumber, ShippingError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ShippingError {
    #[error("carrier {0} does not serve this address")]
    NotServed(String),
    #[error("carrier unreachable: {0}")]
    Unreachable(String),
}
