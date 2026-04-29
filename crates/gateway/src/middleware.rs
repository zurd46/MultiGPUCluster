use axum::{
    extract::{ConnectInfo, Request},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use uuid::Uuid;

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
