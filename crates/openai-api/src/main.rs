use anyhow::Result;
use clap::Parser;
use gpucluster_openai_api::server;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-openai-api", version)]
struct Args {
    #[arg(long, env = "OPENAI_API_BIND", default_value = "0.0.0.0:7200")]
    bind: String,

    /// HTTP base URL of the coordinator (port 7001), not the gRPC bind. Used
    /// to discover live nodes for `/v1/models` and (Phase 2) to dispatch jobs.
    #[arg(long, env = "COORDINATOR_HTTP_URL", default_value = "http://coordinator:7001")]
    coordinator_url: String,

    /// HTTP base URL of mgmt-backend. When set together with `MGMT_API_KEY`,
    /// `/v1/models` is sourced from the live model registry (admin-editable)
    /// instead of a synthetic stub.
    #[arg(long, env = "MGMT_BACKEND_URL", default_value = "http://mgmt:7100")]
    mgmt_url: String,

    /// Service-to-service token for mgmt. We piggyback on the existing
    /// `ADMIN_API_KEY` so there's only one bearer to rotate.
    #[arg(long, env = "ADMIN_API_KEY")]
    mgmt_token: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_openai_api=debug".into()))
        .json()
        .init();

    let args = Args::parse();
    tracing::info!(bind = %args.bind, coordinator = %args.coordinator_url,
        mgmt = %args.mgmt_url, has_mgmt_token = %args.mgmt_token.is_some(),
        "starting openai-api");
    server::run(
        &args.bind,
        &args.coordinator_url,
        Some(args.mgmt_url),
        args.mgmt_token,
    )
    .await
}
