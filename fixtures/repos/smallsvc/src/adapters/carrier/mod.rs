//! Driven adapter over a carrier booking API (external: http).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{Address, Carrier, OrderId, TrackingNumber};
use crate::ports::{ShippingError, ShippingProvider};

pub struct HttpCarrier {
    client: reqwest::Client,
    base_url: String,
}

impl HttpCarrier {
    pub fn new(base_url: String) -> Self {
        Self { client: reqwest::Client::new(), base_url }
    }
}

#[derive(Debug, Serialize)]
struct BookingRequest<'a> {
    reference: String,
    carrier: &'a str,
    to: &'a Address,
}

#[derive(Debug, Deserialize)]
struct BookingResponse {
    tracking: String,
}

#[async_trait]
impl ShippingProvider for HttpCarrier {
    async fn book(&self, order: OrderId, carrier: Carrier, to: &Address) -> Result<TrackingNumber, ShippingError> {
        let url = format!("{}/bookings", self.base_url.trim_end_matches('/'));
        let res = self
            .client
            .post(url)
            .json(&BookingRequest { reference: order.to_string(), carrier: carrier.code(), to })
            .send()
            .await
            .map_err(|e| ShippingError::Unreachable(e.to_string()))?;
        if res.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Err(ShippingError::NotServed(carrier.code().to_string()));
        }
        let booking: BookingResponse = res.json().await.map_err(|e| ShippingError::Unreachable(e.to_string()))?;
        Ok(TrackingNumber(booking.tracking))
    }
}
