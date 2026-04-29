use anyhow::Result;
use axum::middleware as axmid;
use std::net::SocketAddr;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    timeout::TimeoutLayer,
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
    cors::CorsLayer,
};

use crate::{config::GatewayConfig, middleware as gw, routes};

pub async fn run(cfg: GatewayConfig) -> Result<()> {
    let app = routes::build()
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::new(Duration::from_secs(300)))
                .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
                .layer(CorsLayer::permissive())
                .layer(axmid::from_fn(gw::request_id))
                .layer(axmid::from_fn(gw::capture_public_ip)),
        );

    let addr: SocketAddr = cfg.bind.parse()?;
    tracing::info!(%addr, "gateway listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
