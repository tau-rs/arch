use std::sync::Arc;

use crate::domain::{Address, Carrier, OrderId, Shipment};
use crate::ports::{OrderRepository, Outbox, OutboxKind, ShippingProvider};

use super::AppError;

pub struct ShipOrderCommand {
    pub order: OrderId,
    pub carrier: Option<Carrier>,
    pub to: Address,
}

/// The ship flow: load → book with the carrier → mark shipped → persist → enqueue `order_shipped`.
pub struct ShipOrder {
    repo: Arc<dyn OrderRepository>,
    shipping: Arc<dyn ShippingProvider>,
    outbox: Arc<dyn Outbox>,
}

impl ShipOrder {
    pub fn new(repo: Arc<dyn OrderRepository>, shipping: Arc<dyn ShippingProvider>, outbox: Arc<dyn Outbox>) -> Self {
        Self { repo, shipping, outbox }
    }

    pub async fn run(&self, cmd: ShipOrderCommand) -> Result<Shipment, AppError> {
        let mut order = self.repo.get(cmd.order).await?;
        if !order.can_ship() {
            return Err(crate::domain::OrderError::InvalidTransition { from: order.status, to: crate::domain::OrderStatus::Shipped }.into());
        }
        let carrier = cmd.carrier.unwrap_or_else(|| Shipment::default_carrier(&cmd.to));
        let tracking = self.shipping.book(order.id, carrier, &cmd.to).await?;
        let shipment = Shipment::book(order.id, carrier, cmd.to, tracking);
        order.mark_shipped()?;
        self.repo.save(&order).await?;
        self.repo.record_shipment(&shipment).await?;
        self.outbox.enqueue(OutboxKind::OrderShipped, order.id, serde_json::to_value(&shipment).unwrap_or_default()).await?;
        Ok(shipment)
    }
}
