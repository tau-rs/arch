use serde::{Deserialize, Serialize};

use crate::domain::{Address, Carrier, Currency, LineItem, Money, Order, Payment, Shipment};

#[derive(Debug, Deserialize)]
pub struct PlaceOrderRequest {
    pub customer_email: String,
    pub lines: Vec<LineItemDto>,
}

#[derive(Debug, Deserialize)]
pub struct LineItemDto {
    pub sku: String,
    pub quantity: u32,
    pub unit_price_cents: i64,
    pub currency: Currency,
}

impl From<LineItemDto> for LineItem {
    fn from(d: LineItemDto) -> Self {
        LineItem { sku: d.sku, quantity: d.quantity, unit_price: Money::new(d.unit_price_cents, d.currency) }
    }
}

#[derive(Debug, Deserialize)]
pub struct ShipRequest {
    pub carrier: Option<Carrier>,
    pub to: Address,
}

#[derive(Debug, Serialize)]
pub struct OrderResponse {
    pub id: String,
    pub status: String,
    pub total_cents: i64,
    pub currency: String,
    pub lines: usize,
}

impl From<&Order> for OrderResponse {
    fn from(o: &Order) -> Self {
        let total = o.total().ok();
        OrderResponse {
            id: o.id.to_string(),
            status: o.status.as_str().to_string(),
            total_cents: total.map(|m| m.cents).unwrap_or(0),
            currency: total.map(|m| m.currency.code().to_string()).unwrap_or_default(),
            lines: o.lines.len(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PaymentResponse {
    pub id: String,
    pub status: String,
    pub provider_ref: Option<String>,
}

impl From<&Payment> for PaymentResponse {
    fn from(p: &Payment) -> Self {
        PaymentResponse { id: p.id.0.to_string(), status: format!("{:?}", p.status).to_lowercase(), provider_ref: p.provider_ref.clone() }
    }
}

#[derive(Debug, Serialize)]
pub struct ShipmentResponse {
    pub id: String,
    pub carrier: String,
    pub tracking: String,
}

impl From<&Shipment> for ShipmentResponse {
    fn from(s: &Shipment) -> Self {
        ShipmentResponse { id: s.id.0.to_string(), carrier: s.carrier.code().to_string(), tracking: s.tracking.0.clone() }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
