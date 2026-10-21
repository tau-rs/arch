use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::order::{Money, OrderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaymentId(pub Uuid);

impl PaymentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PaymentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Pending,
    Captured,
    Failed,
}

/// A payment attempt against an order, with the provider's reference once captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payment {
    pub id: PaymentId,
    pub order: OrderId,
    pub amount: Money,
    pub status: PaymentStatus,
    pub provider_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Payment {
    pub fn pending(order: OrderId, amount: Money) -> Self {
        Self { id: PaymentId::new(), order, amount, status: PaymentStatus::Pending, provider_ref: None, created_at: Utc::now() }
    }

    pub fn capture(&mut self, provider_ref: String) -> Result<(), PaymentError> {
        if self.status != PaymentStatus::Pending {
            return Err(PaymentError::AlreadySettled(self.status));
        }
        self.status = PaymentStatus::Captured;
        self.provider_ref = Some(provider_ref);
        Ok(())
    }

    pub fn fail(&mut self) {
        self.status = PaymentStatus::Failed;
    }

    pub fn is_captured(&self) -> bool {
        self.status == PaymentStatus::Captured
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaymentError {
    #[error("payment already settled as {0:?}")]
    AlreadySettled(PaymentStatus),
    #[error("provider declined: {0}")]
    Declined(String),
}
