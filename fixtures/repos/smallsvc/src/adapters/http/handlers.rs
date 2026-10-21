use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::app::place::PlaceOrderCommand;
use crate::app::ship::ShipOrderCommand;
use crate::app::AppError;
use crate::domain::OrderId;

use super::dto::{ErrorBody, OrderResponse, PaymentResponse, PlaceOrderRequest, ShipRequest, ShipmentResponse};
use super::AppState;

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = if e.is_not_found() {
            StatusCode::NOT_FOUND
        } else {
            match e {
                AppError::Order(_) | AppError::Payment(_) => StatusCode::UNPROCESSABLE_ENTITY,
                AppError::Gateway(_) | AppError::Shipping(_) | AppError::Notify(_) => StatusCode::BAD_GATEWAY,
                AppError::Repo(_) | AppError::Outbox(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        };
        ApiError(status, e.to_string())
    }
}

fn parse_id(raw: &str) -> Result<OrderId, ApiError> {
    OrderId::parse(raw).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))
}

pub async fn health() -> &'static str {
    "ok"
}

pub async fn place_order(State(state): State<AppState>, Json(req): Json<PlaceOrderRequest>) -> Result<(StatusCode, Json<OrderResponse>), ApiError> {
    let cmd = PlaceOrderCommand { customer_email: req.customer_email, lines: req.lines.into_iter().map(Into::into).collect() };
    let order = state.services.place.run(cmd).await?;
    Ok((StatusCode::CREATED, Json(OrderResponse::from(&order))))
}

pub async fn get_order(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<OrderResponse>, ApiError> {
    let id = parse_id(&id)?;
    let order = state.services.repo.get(id).await.map_err(AppError::from)?;
    Ok(Json(OrderResponse::from(&order)))
}

/// `POST /orders/:id/pay` → `PayOrder::run`.
pub async fn pay_order(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<PaymentResponse>, ApiError> {
    let id = parse_id(&id)?;
    let payment = state.services.pay.run(id).await?;
    Ok(Json(PaymentResponse::from(&payment)))
}

/// `POST /orders/:id/ship` → `ShipOrder::run`.
pub async fn ship_order(State(state): State<AppState>, Path(id): Path<String>, Json(req): Json<ShipRequest>) -> Result<Json<ShipmentResponse>, ApiError> {
    let id = parse_id(&id)?;
    let shipment = state.services.ship.run(ShipOrderCommand { order: id, carrier: req.carrier, to: req.to }).await?;
    Ok(Json(ShipmentResponse::from(&shipment)))
}
