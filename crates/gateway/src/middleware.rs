use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn request_id<B>(mut req: Request<B>, next: Next) -> Response
where
    B: Send + 'static,
{
    let id = Uuid::now_v7().to_string();
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert("x-request-id", id.parse().unwrap());
    resp
}

pub async fn capture_public_ip<B>(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut req: Request<B>,
    next: Next,
) -> Response
where
    B: Send + 'static,
{
    req.extensions_mut().insert(PublicAddr(addr));
    next.run(req).await
}

pub fn security_headers(headers: &mut HeaderMap) {
    headers.insert("strict-transport-security", "max-age=63072000; includeSubDomains".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
}

#[derive(Clone)]
pub struct RequestId(pub String);

#[derive(Clone)]
pub struct PublicAddr(pub SocketAddr);
