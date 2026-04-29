use anyhow::Result;
use clap::Parser;
use gpucluster_openai_api::server;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-openai-api", version)]
struct Args {
    #[arg(long, env = "OPENAI_API_BIND", default_value = "0.0.0.0:7200")]
    bind: String,

    #[arg(long, env = "COORDINATOR_URL", default_value = "http://coordinator:7000")]
    coordinator_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_openai_api=debug".into()))
        .json()
        .init();

    let args = Args::parse();
    tracing::info!(bind = %args.bind, coordinator = %args.coordinator_url, "starting openai-api");
    server::run(&args.bind, &args.coordinator_url).await
}
