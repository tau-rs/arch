//! The outbox worker: dequeues from the shared `outbox` table and hands rows to `NotifyCustomer`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::app::Services;

#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    pub poll: Duration,
    pub batch: i64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { poll: Duration::from_millis(500), batch: 20 }
    }
}

pub async fn run_outbox_worker(services: Arc<Services>, config: WorkerConfig, mut shutdown: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(config.poll);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                match services.notify.drain(config.batch).await {
                    Ok(report) if report.sent + report.retried + report.dropped > 0 => {
                        tracing::info!(?report, "outbox drained");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::error!(error = %e, "outbox drain failed"),
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("outbox worker stopping");
                    return;
                }
            }
        }
    }
}
