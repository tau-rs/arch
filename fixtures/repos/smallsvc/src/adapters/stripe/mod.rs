//! Driven adapter over Stripe's charges API (external: http · api.stripe.com).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{Money, OrderId};
use crate::ports::{GatewayError, PaymentGateway, ProviderRef};

pub struct StripeGateway {
    client: reqwest::Client,
    secret_key: String,
    base_url: String,
}

impl StripeGateway {
    pub fn new(secret_key: String, base_url: String) -> Self {
        Self { client: reqwest::Client::new(), secret_key, base_url }
    }

    fn charges_url(&self) -> String {
        format!("{}/v1/charges", self.base_url.trim_end_matches('/'))
    }

    fn refunds_url(&self) -> String {
        format!("{}/v1/refunds", self.base_url.trim_end_matches('/'))
    }
}

#[derive(Debug, Serialize)]
struct ChargeRequest<'a> {
    amount: i64,
    currency: &'a str,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ChargeResponse {
    id: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct RefundRequest<'a> {
    charge: &'a str,
}

#[derive(Debug, Deserialize)]
struct StripeErrorBody {
    error: StripeErrorDetail,
}

#[derive(Debug, Deserialize)]
struct StripeErrorDetail {
    message: String,
}

#[async_trait]
impl PaymentGateway for StripeGateway {
    async fn charge(&self, order: OrderId, amount: Money) -> Result<ProviderRef, GatewayError> {
        let body = ChargeRequest { amount: amount.cents, currency: amount.currency.code(), description: format!("order {order}") };
        let res = self
            .client
            .post(self.charges_url())
            .bearer_auth(&self.secret_key)
            .form(&body)
            .send()
            .await
            .map_err(|e| GatewayError::Unreachable(e.to_string()))?;
        if res.status().is_client_error() {
            let err: StripeErrorBody = res.json().await.map_err(|e| GatewayError::Protocol(e.to_string()))?;
            return Err(GatewayError::Declined(err.error.message));
        }
        let charge: ChargeResponse = res.json().await.map_err(|e| GatewayError::Protocol(e.to_string()))?;
        if charge.status != "succeeded" {
            return Err(GatewayError::Declined(charge.status));
        }
        Ok(ProviderRef(charge.id))
    }

    async fn refund(&self, provider_ref: &ProviderRef) -> Result<(), GatewayError> {
        self.client
            .post(self.refunds_url())
            .bearer_auth(&self.secret_key)
            .form(&RefundRequest { charge: &provider_ref.0 })
            .send()
            .await
            .map_err(|e| GatewayError::Unreachable(e.to_string()))?
            .error_for_status()
            .map_err(|e| GatewayError::Protocol(e.to_string()))?;
        Ok(())
    }
}
