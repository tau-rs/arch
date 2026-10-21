//! `orderly` binary: wire adapters to ports, serve HTTP, run the outbox worker.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use orderly::adapters::carrier::HttpCarrier;
use orderly::adapters::email::MailApiNotifier;
use orderly::adapters::http::router;
use orderly::adapters::postgres::{self, PgOrderRepository, PgOutbox};
use orderly::adapters::stripe::StripeGateway;
use orderly::app::Services;
use orderly::config::Settings;
use orderly::worker::{run_outbox_worker, WorkerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let settings = Settings::from_env()?;

    let pool = postgres::connect(&settings.database_url).await?;
    postgres::migrate(&pool).await?;

    let services = Arc::new(Services::new(
        Arc::new(PgOrderRepository::new(pool.clone())),
        Arc::new(StripeGateway::new(settings.stripe.secret_key.clone(), settings.stripe.base_url.clone())),
        Arc::new(HttpCarrier::new(settings.carrier_url.clone())),
        Arc::new(MailApiNotifier::new(settings.mail.base_url.clone(), settings.mail.token.clone(), settings.mail.from.clone())),
        Arc::new(PgOutbox::new(pool)),
    ));

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_outbox_worker(
        services.clone(),
        WorkerConfig { poll: Duration::from_millis(settings.outbox_poll_ms), ..WorkerConfig::default() },
        shutdown_rx,
    ));

    let listener = tokio::net::TcpListener::bind(settings.bind).await?;
    tracing::info!(addr = %settings.bind, "orderly listening");
    axum::serve(listener, router(services, &settings.api_key))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    let _ = shutdown_tx.send(true);
    let _ = worker.await;
    Ok(())
}
