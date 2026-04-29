use anyhow::Result;
use clap::Parser;
use gpucluster_gateway::{config::GatewayConfig, server};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "gpucluster-gateway", version)]
struct Args {
    #[arg(long, env = "GATEWAY_BIND", default_value = "0.0.0.0:8443")]
    bind: String,

    #[arg(long, env = "MGMT_BACKEND_URL", default_value = "http://mgmt:7100")]
    mgmt_backend_url: String,

    #[arg(long, env = "COORDINATOR_URL", default_value = "http://coordinator:7000")]
    coordinator_url: String,

    #[arg(long, env = "OPENAI_API_URL", default_value = "http://openai-api:7200")]
    openai_api_url: String,

    #[arg(long, env = "TLS_CERT_PATH")]
    tls_cert: Option<String>,

    #[arg(long, env = "TLS_KEY_PATH")]
    tls_key: Option<String>,

    #[arg(long, env = "CLIENT_CA_PATH")]
    client_ca: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_gateway=debug".into()))
        .json()
        .init();

    let args = Args::parse();
    let cfg = GatewayConfig {
        bind: args.bind,
        mgmt_backend_url: args.mgmt_backend_url,
        coordinator_url: args.coordinator_url,
        openai_api_url: args.openai_api_url,
        tls_cert: args.tls_cert,
        tls_key: args.tls_key,
        client_ca: args.client_ca,
    };

    tracing::info!(?cfg, "starting gateway");
    server::run(cfg).await
}
