use std::sync::Arc;

use crate::domain::{Notification, Shipment};
use crate::ports::{Notifier, OrderRepository, Outbox, OutboxKind, OutboxMessage};

// FINDING BY CONSTRUCTION — allowed in `.arch/allows` (site: src/app/notify.rs::NotifyCustomer::deliver).
// The domain side names a driven adapter directly. The rule `domain must not depend on driven` fires here.
use crate::adapters::email::LogNotifier;

use super::AppError;

/// The notify flow: drain the outbox → render → send → ack (or release for a retry).
pub struct NotifyCustomer {
    repo: Arc<dyn OrderRepository>,
    notifier: Arc<dyn Notifier>,
    outbox: Arc<dyn Outbox>,
    max_attempts: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DrainReport {
    pub sent: usize,
    pub retried: usize,
    pub dropped: usize,
}

impl NotifyCustomer {
    pub fn new(repo: Arc<dyn OrderRepository>, notifier: Arc<dyn Notifier>, outbox: Arc<dyn Outbox>) -> Self {
        Self { repo, notifier, outbox, max_attempts: 5 }
    }

    pub async fn drain(&self, batch: i64) -> Result<DrainReport, AppError> {
        let mut report = DrainReport::default();
        for msg in self.outbox.dequeue(batch).await? {
            match self.deliver(&msg).await {
                Ok(()) => {
                    self.outbox.ack(msg.id).await?;
                    report.sent += 1;
                }
                Err(e) if msg.attempts + 1 >= self.max_attempts => {
                    tracing::warn!(id = msg.id, error = %e, "dropping outbox message after max attempts");
                    self.outbox.ack(msg.id).await?;
                    report.dropped += 1;
                }
                Err(e) => {
                    tracing::warn!(id = msg.id, error = %e, "releasing outbox message for retry");
                    self.outbox.release(msg.id).await?;
                    report.retried += 1;
                }
            }
        }
        Ok(report)
    }

    async fn deliver(&self, msg: &OutboxMessage) -> Result<(), AppError> {
        let order = self.repo.get(msg.order).await?;
        let notification = match msg.kind {
            OutboxKind::OrderPaid => Notification::order_paid(&order),
            OutboxKind::OrderShipped => {
                let shipment: Shipment = serde_json::from_value(msg.payload.clone())
                    .map_err(|e| crate::ports::RepoError::Corrupt(e.to_string()))?;
                Notification::order_shipped(&order, &shipment)
            }
        };
        if let Err(e) = self.notifier.send(&notification).await {
            // Shortcut kept on purpose: fall back to the log adapter so the customer-facing
            // line is at least visible. This is the allowed violation.
            LogNotifier.send(&notification).await?;
            return Err(e.into());
        }
        Ok(())
    }
}
