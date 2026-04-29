use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Response, StatusCode, Uri},
};
use std::sync::Arc;

use crate::state::GatewayState;

#[derive(Clone, Copy)]
pub enum Upstream {
    Mgmt,
    Coordinator,
    OpenAi,
}

impl Upstream {
    fn base<'a>(&self, st: &'a GatewayState) -> &'a str {
        match self {
            Upstream::Mgmt => &st.mgmt_url,
            Upstream::Coordinator => &st.coordinator_http_url,
            Upstream::OpenAi => &st.openai_url,
        }
    }
}

pub async fn forward(
    state: Arc<GatewayState>,
    upstream: Upstream,
    req: Request,
) -> Response<Body> {
    let base = upstream.base(&state);
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let target = format!("{}{}", base.trim_end_matches('/'), path_and_query);

    let target_uri: Uri = match target.parse() {
        Ok(u) => u,
        Err(e) => return bad_gateway(format!("bad upstream uri: {e}")),
    };

    let method = req.method().clone();
    let mut headers = req.headers().clone();
    strip_hop_by_hop(&mut headers);

    let body_bytes = match axum::body::to_bytes(req.into_body(), 50 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return bad_gateway(format!("read body: {e}")),
    };

    let resp = state
        .http
        .request(method, target_uri.to_string())
        .headers(headers)
        .body(body_bytes)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            let mut hdrs = r.headers().clone();
            strip_hop_by_hop(&mut hdrs);
            let bytes = r.bytes().await.unwrap_or_default();
            let mut out = Response::builder().status(status);
            if let Some(h) = out.headers_mut() {
                *h = hdrs;
            }
            out.body(Body::from(bytes))
                .unwrap_or_else(|_| bad_gateway("build response failed"))
        }
        Err(e) => bad_gateway(format!("upstream unreachable: {e}")),
    }
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
    ] {
        headers.remove(name);
    }
}

fn bad_gateway(msg: impl Into<String>) -> Response<Body> {
    let body = serde_json::json!({"error": "bad_gateway", "message": msg.into()}).to_string();
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = StatusCode::BAD_GATEWAY;
    r.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("application/json"),
    );
    r
}

pub async fn proxy_handler(
    State(state): State<Arc<GatewayState>>,
    upstream: Upstream,
    req: Request,
) -> Response<Body> {
    forward(state, upstream, req).await
}
