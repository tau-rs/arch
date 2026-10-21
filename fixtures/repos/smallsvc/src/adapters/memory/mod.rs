//! Driven adapters kept in memory: the second implementation of `OrderRepository` and `Outbox`.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::{Order, OrderId, OrderStatus, Payment, Shipment};
use crate::ports::{OrderRepository, Outbox, OutboxError, OutboxKind, OutboxMessage, RepoError};

#[derive(Default)]
pub struct InMemoryOrderRepository {
    orders: Mutex<HashMap<OrderId, Order>>,
    payments: Mutex<Vec<Payment>>,
    shipments: Mutex<Vec<Shipment>>,
}

impl InMemoryOrderRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn payments(&self) -> Vec<Payment> {
        self.payments.lock().expect("poisoned").clone()
    }

    pub fn shipments(&self) -> Vec<Shipment> {
        self.shipments.lock().expect("poisoned").clone()
    }
}

#[async_trait]
impl OrderRepository for InMemoryOrderRepository {
    async fn get(&self, id: OrderId) -> Result<Order, RepoError> {
        self.orders.lock().expect("poisoned").get(&id).cloned().ok_or(RepoError::NotFound(id))
    }

    async fn save(&self, order: &Order) -> Result<(), RepoError> {
        self.orders.lock().expect("poisoned").insert(order.id, order.clone());
        Ok(())
    }

    async fn list_by_status(&self, status: OrderStatus) -> Result<Vec<Order>, RepoError> {
        Ok(self.orders.lock().expect("poisoned").values().filter(|o| o.status == status).cloned().collect())
    }

    async fn record_payment(&self, payment: &Payment) -> Result<(), RepoError> {
        self.payments.lock().expect("poisoned").push(payment.clone());
        Ok(())
    }

    async fn record_shipment(&self, shipment: &Shipment) -> Result<(), RepoError> {
        self.shipments.lock().expect("poisoned").push(shipment.clone());
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryOutbox {
    next_id: Mutex<i64>,
    rows: Mutex<Vec<(OutboxMessage, bool)>>,
}

impl InMemoryOutbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> usize {
        self.rows.lock().expect("poisoned").len()
    }
}

#[async_trait]
impl Outbox for InMemoryOutbox {
    async fn enqueue(&self, kind: OutboxKind, order: OrderId, payload: serde_json::Value) -> Result<(), OutboxError> {
        let mut next = self.next_id.lock().expect("poisoned");
        *next += 1;
        self.rows.lock().expect("poisoned").push((OutboxMessage { id: *next, kind, order, payload, attempts: 0 }, false));
        Ok(())
    }

    async fn dequeue(&self, limit: i64) -> Result<Vec<OutboxMessage>, OutboxError> {
        let mut rows = self.rows.lock().expect("poisoned");
        let mut out = Vec::new();
        for (msg, locked) in rows.iter_mut().filter(|(_, locked)| !*locked).take(limit.max(0) as usize) {
            *locked = true;
            msg.attempts += 1;
            out.push(msg.clone());
        }
        Ok(out)
    }

    async fn ack(&self, id: i64) -> Result<(), OutboxError> {
        self.rows.lock().expect("poisoned").retain(|(m, _)| m.id != id);
        Ok(())
    }

    async fn release(&self, id: i64) -> Result<(), OutboxError> {
        if let Some((_, locked)) = self.rows.lock().expect("poisoned").iter_mut().find(|(m, _)| m.id == id) {
            *locked = false;
        }
        Ok(())
    }
}
