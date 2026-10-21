//! Ports: the traits the domain side needs fulfilled. Adapters implement them; nothing here does I/O.

pub mod notifier;
pub mod order_repo;
pub mod outbox;
pub mod payment_gateway;
pub mod shipping;

pub use notifier::{Notifier, NotifyError};
pub use order_repo::{OrderRepository, RepoError};
pub use outbox::{Outbox, OutboxError, OutboxKind, OutboxMessage};
pub use payment_gateway::{GatewayError, PaymentGateway, ProviderRef};
pub use shipping::{ShippingError, ShippingProvider};
