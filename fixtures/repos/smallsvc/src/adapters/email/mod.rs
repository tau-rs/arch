//! Driven adapters for notifications: a mail API over HTTP, and a log fallback (external: tty).

use async_trait::async_trait;
use serde::Serialize;

use crate::domain::{Channel, Notification};
use crate::ports::{Notifier, NotifyError};

pub struct MailApiNotifier {
    client: reqwest::Client,
    base_url: String,
    token: String,
    from: String,
}

impl MailApiNotifier {
    pub fn new(base_url: String, token: String, from: String) -> Self {
        Self { client: reqwest::Client::new(), base_url, token, from }
    }
}

#[derive(Debug, Serialize)]
struct SendRequest<'a> {
    #[serde(rename = "From")]
    from: &'a str,
    #[serde(rename = "To")]
    to: &'a str,
    #[serde(rename = "Subject")]
    subject: &'a str,
    #[serde(rename = "TextBody")]
    body: &'a str,
}

#[async_trait]
impl Notifier for MailApiNotifier {
    async fn send(&self, n: &Notification) -> Result<(), NotifyError> {
        if n.channel != Channel::Email {
            return Err(NotifyError::Rejected(format!("channel {:?} unsupported", n.channel)));
        }
        let url = format!("{}/email", self.base_url.trim_end_matches('/'));
        self.client
            .post(url)
            .header("X-Postmark-Server-Token", &self.token)
            .json(&SendRequest { from: &self.from, to: &n.to, subject: &n.subject, body: &n.body })
            .send()
            .await
            .map_err(|e| NotifyError::Unreachable(e.to_string()))?
            .error_for_status()
            .map_err(|e| NotifyError::Rejected(e.to_string()))?;
        Ok(())
    }
}

/// Writes the notification to the process log instead of sending it.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogNotifier;

#[async_trait]
impl Notifier for LogNotifier {
    async fn send(&self, n: &Notification) -> Result<(), NotifyError> {
        tracing::info!(to = %n.to, subject = %n.subject, "notification (log only)");
        Ok(())
    }
}
