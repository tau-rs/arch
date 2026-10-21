//! Driving adapter: axum routes → handlers, behind a middleware stack.

pub mod dto;
pub mod handlers;
pub mod middleware;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::app::Services;

#[derive(Clone)]
pub struct AppState {
    pub services: Arc<Services>,
    pub api_key: Arc<str>,
}

/// The route table. Middleware stack, outermost first: trace → request id → api key.
pub fn router(services: Arc<Services>, api_key: &str) -> Router {
    let state = AppState { services, api_key: Arc::from(api_key) };
    Router::new()
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        .route("/orders/:id/pay", post(handlers::pay_order))
        .route("/orders/:id/ship", post(handlers::ship_order))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), middleware::require_api_key))
        .route("/health", get(handlers::health))
        .layer(axum::middleware::from_fn(middleware::request_id))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
