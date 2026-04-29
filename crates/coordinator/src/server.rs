use anyhow::Result;
use axum::{routing::get, Json, Router};
use serde_json::json;
use std::net::SocketAddr;
use tonic::transport::Server;

use crate::{
    config::CoordConfig,
    heartbeat,
    registry::Registry,
    service::CoordSvc,
};
use gpucluster_proto::coordinator::coordinator_service_server::CoordinatorServiceServer;

pub async fn run(cfg: CoordConfig) -> Result<()> {
    let registry = Registry::new();

    tokio::spawn(heartbeat::watchdog(registry.clone()));

    let http = run_http(&cfg.http_bind, registry.clone());
    let grpc = run_grpc(&cfg.grpc_bind, registry.clone());

    tokio::try_join!(http, grpc)?;
    Ok(())
}

async fn run_http(bind: &str, reg: Registry) -> Result<()> {
    let app = Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/nodes",  get({
            let reg = reg.clone();
            move || async move {
                let nodes: Vec<_> = reg.list().into_iter().map(|e| {
                    json!({
                        "id": e.info.node_id,
                        "hostname": e.info.hostname,
                        "status": e.info.status,
                        "last_heartbeat": e.last_heartbeat.to_rfc3339(),
                    })
                }).collect();
                Json(json!({ "nodes": nodes }))
            }
        }));
    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, "coordinator http listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_grpc(bind: &str, reg: Registry) -> Result<()> {
    let svc = CoordSvc { registry: reg };
    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, "coordinator grpc listening");
    Server::builder()
        .add_service(CoordinatorServiceServer::new(svc))
        .serve(addr)
        .await?;
    Ok(())
}
