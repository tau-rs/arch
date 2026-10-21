use async_trait::async_trait;

use crate::domain::Notification;

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("rejected by provider: {0}")]
    Rejected(String),
    #[error("provider unreachable: {0}")]
    Unreachable(String),
}
