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
    database_url: Option<String>,

    #[arg(long, env = "JWT_SECRET")]
    jwt_secret: Option<String>,
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
    };

    tracing::info!(bind = %cfg.bind, "starting mgmt-backend");
    server::run(cfg).await
}
