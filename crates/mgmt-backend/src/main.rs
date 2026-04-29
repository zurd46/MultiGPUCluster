use anyhow::Result;
use clap::Parser;
use gpucluster_mgmt_backend::{config::MgmtConfig, server};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-mgmt", version)]
struct Args {
    #[arg(long, env = "MGMT_BIND", default_value = "0.0.0.0:7100")]
    bind: String,

    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    #[arg(long, env = "JWT_SECRET")]
    jwt_secret: Option<String>,

    /// Shared bearer-token for admin endpoints (Phase 1).
    /// In production this is replaced by OAuth/OIDC + scoped API keys.
    #[arg(long, env = "ADMIN_API_KEY")]
    admin_api_key: Option<String>,

    #[arg(long, env = "COORDINATOR_ENDPOINT", default_value = "https://localhost/cluster")]
    coordinator_endpoint: String,

    #[arg(long, env = "CA_COMMON_NAME", default_value = "MultiGPUCluster Root CA")]
    ca_common_name: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_mgmt_backend=debug".into()))
        .json()
        .init();

    let args = Args::parse();
    let cfg = MgmtConfig {
        bind: args.bind,
        database_url: args.database_url,
        jwt_secret: args.jwt_secret.unwrap_or_else(|| "dev-only-change-me".into()),
        admin_api_key: args.admin_api_key.unwrap_or_else(|| {
            tracing::warn!("ADMIN_API_KEY not set — using insecure dev default 'dev-admin-key'");
            "dev-admin-key".into()
        }),
        coordinator_endpoint: args.coordinator_endpoint,
        ca_common_name: args.ca_common_name,
    };

    tracing::info!(bind = %cfg.bind, "starting mgmt-backend");
    server::run(cfg).await
}
