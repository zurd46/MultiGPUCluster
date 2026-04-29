use anyhow::Result;
use clap::Parser;
use gpucluster_coordinator::{config::CoordConfig, server};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-coordinator", version)]
struct Args {
    #[arg(long, env = "COORD_GRPC_BIND", default_value = "0.0.0.0:7000")]
    grpc_bind: String,

    #[arg(long, env = "COORD_HTTP_BIND", default_value = "0.0.0.0:7001")]
    http_bind: String,

    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_coordinator=debug".into()))
        .json()
        .init();

    let args = Args::parse();
    let cfg = CoordConfig {
        grpc_bind: args.grpc_bind,
        http_bind: args.http_bind,
        database_url: args.database_url,
    };

    tracing::info!(?cfg, "starting coordinator");
    server::run(cfg).await
}
