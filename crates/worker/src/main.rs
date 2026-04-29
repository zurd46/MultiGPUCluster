use anyhow::Result;
use clap::Parser;
use gpucluster_worker::{config::WorkerConfig, agent};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-worker", version)]
struct Args {
    #[arg(long, env = "COORDINATOR_URL", default_value = "http://coordinator:7000")]
    coordinator_url: String,

    #[arg(long, env = "NODE_DATA_DIR", default_value = "/var/lib/gpucluster")]
    data_dir: String,

    #[arg(long, env = "NODE_DISPLAY_NAME")]
    display_name: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_worker=debug".into()))
        .init();

    let args = Args::parse();
    let cfg = WorkerConfig {
        coordinator_url: args.coordinator_url,
        data_dir: args.data_dir,
        display_name: args.display_name,
    };

    tracing::info!(?cfg, "starting worker");
    agent::run(cfg).await
}
