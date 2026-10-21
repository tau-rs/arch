use serde::{Deserialize, Serialize};

use super::order::Order;
use super::shipment::Shipment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Email,
    Sms,
}

/// A message to a customer, rendered from a domain event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub to: String,
    pub channel: Channel,
    pub subject: String,
    pub body: String,
}

impl Notification {
    pub fn order_paid(order: &Order) -> Self {
        Self {
            to: order.customer_email.clone(),
            channel: Channel::Email,
            subject: format!("Order {} confirmed", order.id),
            body: render(Template::Paid, order, None),
        }
    }

    pub fn order_shipped(order: &Order, shipment: &Shipment) -> Self {
        Self {
            to: order.customer_email.clone(),
            channel: Channel::Email,
            subject: format!("Order {} shipped", order.id),
            body: render(Template::Shipped, order, Some(shipment)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Template {
    Paid,
    Shipped,
}

fn render(template: Template, order: &Order, shipment: Option<&Shipment>) -> String {
    match (template, shipment) {
        (Template::Paid, _) => format!("Thanks! We received payment for {} line(s).", order.lines.len()),
        (Template::Shipped, Some(s)) => format!("Your order left with {} — tracking {}.", s.carrier.code(), s.tracking.0),
        (Template::Shipped, None) => "Your order is on its way.".to_string(),
    }
}
