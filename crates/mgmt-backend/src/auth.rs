use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};

use crate::{api_error::ApiError, state::AppState};

/// Bearer-token middleware for admin endpoints.
/// Phase 1: single shared admin key (env ADMIN_API_KEY).
/// Phase 1.x: replace with per-user OAuth/OIDC + scoped API keys.
pub async fn require_admin(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(hdr) = req.headers().get(AUTHORIZATION) else {
        return Err(ApiError::Unauthorized);
    };
    let val = hdr.to_str().map_err(|_| ApiError::Unauthorized)?;
    let Some(token) = val.strip_prefix("Bearer ") else {
        return Err(ApiError::Unauthorized);
    };
    if !constant_time_eq(token.as_bytes(), s.admin_api_key.as_bytes()) {
        return Err(ApiError::Unauthorized);
    }
    Ok(next.run(req).await)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
