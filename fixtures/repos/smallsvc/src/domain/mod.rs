//! The domain: orders, payments, shipments, notifications. No I/O, no framework types.

pub mod notification;
pub mod order;
pub mod payment;
pub mod shipment;

pub use notification::{Channel, Notification};
pub use order::{Currency, LineItem, Money, Order, OrderError, OrderId, OrderStatus};
pub use payment::{Payment, PaymentError, PaymentId, PaymentStatus};
pub use shipment::{Address, Carrier, Shipment, ShipmentId, ShipmentStatus, TrackingNumber};
