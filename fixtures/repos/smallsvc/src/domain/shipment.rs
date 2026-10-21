use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::order::OrderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShipmentId(pub Uuid);

impl ShipmentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ShipmentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    Ups,
    Dhl,
    Local,
}

impl Carrier {
    pub fn code(self) -> &'static str {
        match self {
            Carrier::Ups => "ups",
            Carrier::Dhl => "dhl",
            Carrier::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ups" => Carrier::Ups,
            "dhl" => Carrier::Dhl,
            "local" => Carrier::Local,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub line1: String,
    pub city: String,
    pub postcode: String,
    pub country: String,
}

impl Address {
    pub fn is_domestic(&self) -> bool {
        self.country.eq_ignore_ascii_case("FR")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackingNumber(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShipmentStatus {
    Booked,
    InTransit,
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shipment {
    pub id: ShipmentId,
    pub order: OrderId,
    pub carrier: Carrier,
    pub to: Address,
    pub tracking: TrackingNumber,
    pub status: ShipmentStatus,
    pub booked_at: DateTime<Utc>,
}

impl Shipment {
    pub fn book(order: OrderId, carrier: Carrier, to: Address, tracking: TrackingNumber) -> Self {
        Self { id: ShipmentId::new(), order, carrier, to, tracking, status: ShipmentStatus::Booked, booked_at: Utc::now() }
    }

    pub fn advance(&mut self) {
        self.status = match self.status {
            ShipmentStatus::Booked => ShipmentStatus::InTransit,
            ShipmentStatus::InTransit | ShipmentStatus::Delivered => ShipmentStatus::Delivered,
        };
    }

    /// Pick a carrier when the caller did not: local for domestic, DHL abroad.
    pub fn default_carrier(to: &Address) -> Carrier {
        if to.is_domestic() {
            Carrier::Local
        } else {
            Carrier::Dhl
        }
    }
}
