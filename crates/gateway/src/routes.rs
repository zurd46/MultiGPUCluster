use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware as axmid,
    response::{Html, IntoResponse, Response},
    routing::{any, get},
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    middleware as gw,
    proxy::{forward, Upstream},
    state::GatewayState,
};

const ADMIN_HTML: &str = include_str!("admin_ui.html");

pub fn build(state: Arc<GatewayState>) -> Router {
    // Customer-facing OpenAI-compatible API. Bearer token (mgc_*) required.
    // The auth middleware lives on its own sub-router so it doesn't apply to
    // /api/* (admin) or /cluster/* (worker) which have their own auth schemes.
    let v1 = Router::new()
        .route("/v1/{*rest}", any(openai_proxy))
        .route_layer(axmid::from_fn_with_state(
            state.clone(),
            gw::require_v1_api_key,
        ));

    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/overview", get(overview))
        // Reverse-proxy fan-out — axum 0.8 wildcard syntax: {*rest}
        .route("/api/{*rest}", any(api_proxy))
        .route("/cluster/{*rest}", any(cluster_proxy))
        .route("/enroll", any(enroll_proxy))
        .merge(v1)
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn ready() -> Json<Value> {
    Json(json!({ "ready": true }))
}

// ---------- proxy handlers ----------

async fn api_proxy(State(s): State<Arc<GatewayState>>, req: Request) -> Response {
    forward(s, Upstream::Mgmt, req).await
}

async fn openai_proxy(State(s): State<Arc<GatewayState>>, req: Request) -> Response {
    forward(s, Upstream::OpenAi, req).await
}

async fn cluster_proxy(State(s): State<Arc<GatewayState>>, req: Request) -> Response {
    // /cluster/foo  →  /foo on coordinator HTTP
    let mut req = req;
    let new_uri = {
        let uri = req.uri();
        let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let stripped = pq.strip_prefix("/cluster").unwrap_or(pq);
        let stripped = if stripped.is_empty() { "/" } else { stripped };
        stripped.parse().unwrap_or_else(|_| uri.clone())
    };
    *req.uri_mut() = new_uri;
    forward(s, Upstream::Coordinator, req).await
}

async fn enroll_proxy(State(s): State<Arc<GatewayState>>, mut req: Request) -> Response {
    // /enroll → /api/v1/enroll on mgmt-backend
    let new_uri = "/api/v1/enroll".parse().unwrap();
    *req.uri_mut() = new_uri;
    forward(s, Upstream::Mgmt, req).await
}

// ---------- aggregation ----------

async fn overview(State(s): State<Arc<GatewayState>>, headers: HeaderMap) -> impl IntoResponse {
    let mgmt_url = s.mgmt_url.trim_end_matches('/').to_string();
    let coord_url = s.coordinator_http_url.trim_end_matches('/').to_string();
    let openai_url = s.openai_url.trim_end_matches('/').to_string();

    // bearer pass-through for mgmt admin endpoints
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    let url_mgmt_health   = format!("{mgmt_url}/health");
    let url_coord_health  = format!("{coord_url}/health");
    let url_openai_health = format!("{openai_url}/health");
    let url_coord_nodes   = format!("{coord_url}/nodes");
    let url_mgmt_nodes    = format!("{mgmt_url}/api/v1/nodes");
    let url_openai_models = format!("{openai_url}/v1/models");

    // health checks (parallel)
    let h_mgmt   = check_health(&s.http, &url_mgmt_health);
    let h_coord  = check_health(&s.http, &url_coord_health);
    let h_openai = check_health(&s.http, &url_openai_health);

    // payloads (parallel)
    let p_coord  = fetch_json(&s.http, &url_coord_nodes, None);
    let p_mgmt   = fetch_json(&s.http, &url_mgmt_nodes, bearer.as_deref());
    let p_models = fetch_json(&s.http, &url_openai_models, None);

    let (m, c, o, coord_nodes, mgmt_nodes, models) =
        tokio::join!(h_mgmt, h_coord, h_openai, p_coord, p_mgmt, p_models);

    let body = json!({
        "services": {
            "gateway":     { "status": "ok" },
            "mgmt":        { "status": status_str(m) },
            "coordinator": { "status": status_str(c) },
            "openai_api":  { "status": status_str(o) },
        },
        "coordinator": coord_nodes,
        "mgmt":        mgmt_nodes,
        "openai_api":  models,
    });

    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    resp
}

async fn check_health(http: &reqwest::Client, url: &str) -> bool {
    matches!(http.get(url).send().await, Ok(r) if r.status().is_success())
}

async fn fetch_json(http: &reqwest::Client, url: &str, bearer: Option<&str>) -> Value {
    let mut req = http.get(url);
    if let Some(b) = bearer {
        req = req.header("authorization", b);
    }
    match req.send().await {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                r.json::<Value>().await.unwrap_or_else(|e| {
                    json!({ "error": "invalid_json", "message": e.to_string() })
                })
            } else {
                json!({
                    "error": "upstream_status",
                    "status": status.as_u16(),
                })
            }
        }
        Err(e) => json!({ "error": "unreachable", "message": e.to_string() }),
    }
}

fn status_str(ok: bool) -> &'static str {
    if ok { "ok" } else { "down" }
}
