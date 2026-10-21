use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use super::AppState;

pub const API_KEY_HEADER: &str = "x-api-key";
pub const REQUEST_ID_HEADER: &str = "x-request-id";

pub async fn require_api_key(State(state): State<AppState>, req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let presented = req.headers().get(API_KEY_HEADER).and_then(|v| v.to_str().ok());
    if presented != Some(&*state.api_key) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

pub async fn request_id(req: Request<Body>, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut res = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        res.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    res
}
