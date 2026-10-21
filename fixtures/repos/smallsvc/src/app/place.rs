use std::sync::Arc;

use crate::domain::{LineItem, Order};
use crate::ports::OrderRepository;

use super::AppError;

pub struct PlaceOrderCommand {
    pub customer_email: String,
    pub lines: Vec<LineItem>,
}

pub struct PlaceOrder {
    repo: Arc<dyn OrderRepository>,
}

impl PlaceOrder {
    pub fn new(repo: Arc<dyn OrderRepository>) -> Self {
        Self { repo }
    }

    pub async fn run(&self, cmd: PlaceOrderCommand) -> Result<Order, AppError> {
        let order = Order::place(cmd.customer_email, cmd.lines)?;
        self.repo.save(&order).await?;
        tracing::info!(order = %order.id, "order placed");
        Ok(order)
    }
}
