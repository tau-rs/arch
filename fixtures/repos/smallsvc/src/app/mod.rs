//! Use cases. Each one takes ports, never adapters — with one allowed exception in `notify`.

pub mod notify;
pub mod pay;
pub mod place;
pub mod ship;

use std::sync::Arc;

use crate::ports::{Notifier, OrderRepository, Outbox, PaymentGateway, ShippingProvider};

pub use notify::NotifyCustomer;
pub use pay::PayOrder;
pub use place::PlaceOrder;
pub use ship::ShipOrder;

/// The use cases wired to their ports; the HTTP adapter and the worker share one instance.
pub struct Services {
    pub repo: Arc<dyn OrderRepository>,
    pub place: PlaceOrder,
    pub pay: PayOrder,
    pub ship: ShipOrder,
    pub notify: NotifyCustomer,
}

impl Services {
    pub fn new(
        repo: Arc<dyn OrderRepository>,
        gateway: Arc<dyn PaymentGateway>,
        shipping: Arc<dyn ShippingProvider>,
        notifier: Arc<dyn Notifier>,
        outbox: Arc<dyn Outbox>,
    ) -> Self {
        Self {
            repo: repo.clone(),
            place: PlaceOrder::new(repo.clone()),
            pay: PayOrder::new(repo.clone(), gateway, outbox.clone()),
            ship: ShipOrder::new(repo.clone(), shipping, outbox.clone()),
            notify: NotifyCustomer::new(repo, notifier, outbox),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Order(#[from] crate::domain::OrderError),
    #[error(transparent)]
    Payment(#[from] crate::domain::PaymentError),
    #[error(transparent)]
    Repo(#[from] crate::ports::RepoError),
    #[error(transparent)]
    Gateway(#[from] crate::ports::GatewayError),
    #[error(transparent)]
    Shipping(#[from] crate::ports::ShippingError),
    #[error(transparent)]
    Notify(#[from] crate::ports::NotifyError),
    #[error(transparent)]
    Outbox(#[from] crate::ports::OutboxError),
}

impl AppError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, AppError::Repo(crate::ports::RepoError::NotFound(_)) | AppError::Order(crate::domain::OrderError::NotFound(_)))
    }
}
