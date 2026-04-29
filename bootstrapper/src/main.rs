use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};

mod enroll;
mod service;
mod docker;
mod preflight;
mod state;

#[derive(Parser, Debug)]
#[command(name = "gpucluster-agent", version, about = "MultiGPUCluster host agent")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Install host service (systemd unit / Windows Service) and prerequisites check
    Install,
    /// Uninstall service, remove containers, optionally drop local data
    Uninstall {
        #[arg(long)]
        purge: bool,
    },
    /// One-time enrollment with the backend
    Enroll {
        #[arg(long)]
        backend: String,
        #[arg(long)]
        token: String,
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Run the persistent agent loop (typically invoked by service manager)
    Run,
    /// Show local agent status
    Status,
    /// Force a re-enrollment with a new token
    ReEnroll {
        #[arg(long)]
        backend: String,
        #[arg(long)]
        token: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gpucluster_agent=debug".into()))
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Install                          => service::install().await,
        Cmd::Uninstall { purge }              => service::uninstall(purge).await,
        Cmd::Enroll { backend, token, display_name } => enroll::run(&backend, &token, display_name.as_deref()).await,
        Cmd::Run                              => service::run_loop().await,
        Cmd::Status                           => service::status().await,
        Cmd::ReEnroll { backend, token }      => enroll::run(&backend, &token, None).await,
    }
}
