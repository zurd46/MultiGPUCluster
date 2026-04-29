use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{header::AUTHORIZATION, HeaderMap, HeaderValue, Response as HttpResponse, StatusCode},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::state::{CachedAuth, GatewayState};

pub async fn request_id(mut req: Request, next: Next) -> Response {
    let id = Uuid::now_v7().to_string();
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut resp = next.run(req).await;
    if let Ok(val) = id.parse() {
        resp.headers_mut().insert("x-request-id", val);
    }
    resp
}

pub async fn capture_public_ip(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Response {
    req.extensions_mut().insert(PublicAddr(addr));
    next.run(req).await
}

pub fn security_headers(headers: &mut HeaderMap) {
    if let Ok(v) = "max-age=63072000; includeSubDomains".parse() {
        headers.insert("strict-transport-security", v);
    }
    if let Ok(v) = "nosniff".parse() {
        headers.insert("x-content-type-options", v);
    }
    if let Ok(v) = "DENY".parse() {
        headers.insert("x-frame-options", v);
    }
    if let Ok(v) = "no-referrer".parse() {
        headers.insert("referrer-policy", v);
    }
}

#[derive(Clone)]
pub struct RequestId(pub String);

#[derive(Clone)]
pub struct PublicAddr(pub SocketAddr);

/// Identifies the customer API key the current /v1/* request was authenticated
/// with. Stored on the request so downstream handlers (e.g. usage tracking)
/// can attribute work without re-parsing the bearer header.
#[derive(Clone, Debug)]
pub struct AuthedKey {
    pub key_id: String,
    pub name: String,
    pub scope: String,
}

/// Bearer-token auth for /v1/*. Reads `Authorization: Bearer <token>` from the
/// request, asks mgmt-backend to verify it, and caches the verdict.
///
/// Failure modes:
///   * missing/malformed header → 401
///   * mgmt-backend unreachable → 503 (we deliberately do *not* fail open)
///   * verdict.ok == false      → 401
pub async fn require_v1_api_key(
    State(state): State<Arc<GatewayState>>,
    req: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer(req.headers()) {
        Some(t) => t,
        None => return unauthorized("missing Authorization: Bearer <token>"),
    };

    // Cheap structural check: matches what mgmt-backend issues. Saves a
    // round-trip on obviously bad inputs (e.g. someone using ADMIN_API_KEY
    // here by mistake).
    if !token.starts_with("mgc_") || token.len() < 16 {
        return unauthorized("invalid api key format (expected mgc_*)");
    }

    if let Some(cached) = state.auth_cache.get(&token).map(|e| e.value().clone()) {
        if cached.fresh() {
            return apply_verdict(cached, req, next).await;
        }
    }

    let verdict = match verify_with_mgmt(&state, &token).await {
        Ok(v) => v,
        Err(msg) => {
            tracing::warn!(error = %msg, "/v1/* auth: mgmt unreachable");
            return service_unavailable("auth backend unreachable");
        }
    };

    state.auth_cache.insert(token.clone(), verdict.clone());

    // Periodic eviction so the map can't grow without bound. ~1k entries is
    // ~50 KB, so we only sweep when we cross that threshold.
    if state.auth_cache.len() > 1024 {
        state.auth_cache.retain(|_, v| v.fresh());
    }

    apply_verdict(verdict, req, next).await
}

async fn apply_verdict(verdict: CachedAuth, mut req: Request, next: Next) -> Response {
    if !verdict.ok {
        return unauthorized("invalid or revoked api key");
    }
    if let (Some(id), Some(name), Some(scope)) =
        (verdict.key_id, verdict.name, verdict.scope)
    {
        req.extensions_mut().insert(AuthedKey {
            key_id: id,
            name,
            scope,
        });
    }
    next.run(req).await
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(AUTHORIZATION)?.to_str().ok()?;
    v.strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn verify_with_mgmt(state: &GatewayState, token: &str) -> Result<CachedAuth, String> {
    let admin_key = state
        .admin_api_key
        .as_deref()
        .ok_or_else(|| "ADMIN_API_KEY not configured on gateway".to_string())?;

    let url = format!(
        "{}/api/v1/keys/verify",
        state.mgmt_url.trim_end_matches('/')
    );
    let res = state
        .http
        .post(url)
        .bearer_auth(admin_key)
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("mgmt verify returned HTTP {}", res.status()));
    }
    let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(CachedAuth {
        ok,
        key_id: body
            .get("key_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        name: body.get("name").and_then(|v| v.as_str()).map(String::from),
        scope: body.get("scope").and_then(|v| v.as_str()).map(String::from),
        fetched_at: Instant::now(),
    })
}

fn unauthorized(msg: &str) -> Response {
    json_error(StatusCode::UNAUTHORIZED, "unauthorized", msg)
}

fn service_unavailable(msg: &str) -> Response {
    json_error(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", msg)
}

fn json_error(status: StatusCode, kind: &str, msg: &str) -> Response {
    let body = serde_json::json!({ "error": kind, "message": msg }).to_string();
    let mut r = HttpResponse::new(Body::from(body));
    *r.status_mut() = status;
    r.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("application/json"),
    );
    r
}
