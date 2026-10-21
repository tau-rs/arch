use std::sync::Arc;

use crate::domain::{Order, OrderId, Payment};
use crate::ports::{OrderRepository, Outbox, OutboxKind, PaymentGateway};

use super::AppError;

/// The pay flow: load → charge the gateway → mark paid → persist → enqueue `order_paid`.
pub struct PayOrder {
    repo: Arc<dyn OrderRepository>,
    gateway: Arc<dyn PaymentGateway>,
    outbox: Arc<dyn Outbox>,
}

impl PayOrder {
    pub fn new(repo: Arc<dyn OrderRepository>, gateway: Arc<dyn PaymentGateway>, outbox: Arc<dyn Outbox>) -> Self {
        Self { repo, gateway, outbox }
    }

    pub async fn run(&self, id: OrderId) -> Result<Payment, AppError> {
        let mut order = self.repo.get(id).await?;
        let amount = order.total()?;
        let mut payment = Payment::pending(order.id, amount);
        match self.gateway.charge(order.id, amount).await {
            Ok(provider_ref) => payment.capture(provider_ref.0)?,
            Err(e) => {
                payment.fail();
                self.repo.record_payment(&payment).await?;
                return Err(e.into());
            }
        }
        order.mark_paid()?;
        self.repo.save(&order).await?;
        self.repo.record_payment(&payment).await?;
        self.outbox.enqueue(OutboxKind::OrderPaid, order.id, paid_payload(&order, &payment)).await?;
        Ok(payment)
    }
}

fn paid_payload(order: &Order, payment: &Payment) -> serde_json::Value {
    serde_json::json!({ "order": order.id, "payment": payment.id, "amount": payment.amount })
}
