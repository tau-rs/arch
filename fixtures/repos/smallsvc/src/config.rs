//! Process configuration, read from the environment once at start.

use std::env;
use std::net::SocketAddr;

/// Everything `main` needs to wire the service.
#[derive(Debug, Clone)]
pub struct Settings {
    pub bind: SocketAddr,
    pub database_url: String,
    pub api_key: String,
    pub stripe: StripeSettings,
    pub carrier_url: String,
    pub mail: MailSettings,
    pub outbox_poll_ms: u64,
}

#[derive(Debug, Clone)]
pub struct StripeSettings {
    pub secret_key: String,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub struct MailSettings {
    pub base_url: String,
    pub token: String,
    pub from: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value for {0}: {1}")]
    Invalid(&'static str, String),
}

impl Settings {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind = var_or("ORDERLY_BIND", "127.0.0.1:8080")
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::Invalid("ORDERLY_BIND", e.to_string()))?;
        let outbox_poll_ms = var_or("ORDERLY_OUTBOX_POLL_MS", "500")
            .parse()
            .map_err(|e: std::num::ParseIntError| ConfigError::Invalid("ORDERLY_OUTBOX_POLL_MS", e.to_string()))?;
        Ok(Self {
            bind,
            database_url: required("DATABASE_URL")?,
            api_key: required("ORDERLY_API_KEY")?,
            stripe: StripeSettings {
                secret_key: required("STRIPE_SECRET_KEY")?,
                base_url: var_or("STRIPE_BASE_URL", "https://api.stripe.com"),
            },
            carrier_url: var_or("CARRIER_BASE_URL", "https://api.carrier.example"),
            mail: MailSettings {
                base_url: var_or("MAIL_BASE_URL", "https://api.postmarkapp.com"),
                token: var_or("MAIL_TOKEN", ""),
                from: var_or("MAIL_FROM", "orders@orderly.example"),
            },
            outbox_poll_ms,
        })
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::Missing(name))
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}
