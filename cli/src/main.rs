use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "gpucluster", version, about = "MultiGPUCluster admin CLI")]
struct Cli {
    #[arg(long, env = "GPUCLUSTER_BACKEND", default_value = "https://localhost:8443")]
    backend: String,

    #[arg(long, env = "GPUCLUSTER_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Manage cluster nodes
    Nodes {
        #[command(subcommand)]
        cmd: NodesCmd,
    },
    /// Manage inference / fine-tune jobs
    Jobs {
        #[command(subcommand)]
        cmd: JobsCmd,
    },
    /// Show cluster status
    Status,
}

#[derive(Subcommand, Debug)]
enum NodesCmd {
    List,
    Show   { id: String },
    Approve{ id: String },
    Revoke { id: String, #[arg(long)] reason: Option<String> },
    Drain  { id: String },
    Token  { #[arg(long)] display: Option<String> },
}

#[derive(Subcommand, Debug)]
enum JobsCmd {
    List,
    Show   { id: String },
    Submit { #[arg(long)] spec: String },
    Cancel { id: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Status => println!("backend = {}", cli.backend),
        Cmd::Nodes { cmd } => match cmd {
            NodesCmd::List          => println!("[stub] GET {}/api/v1/nodes", cli.backend),
            NodesCmd::Show { id }   => println!("[stub] GET {}/api/v1/nodes/{id}", cli.backend),
            NodesCmd::Approve{ id } => println!("[stub] POST {}/api/v1/nodes/{id}/approve", cli.backend),
            NodesCmd::Revoke{ id, .. } => println!("[stub] POST {}/api/v1/nodes/{id}/revoke", cli.backend),
            NodesCmd::Drain { id }  => println!("[stub] POST {}/api/v1/nodes/{id}/drain", cli.backend),
            NodesCmd::Token { .. }  => println!("[stub] POST {}/api/v1/enroll/tokens", cli.backend),
        },
        Cmd::Jobs { cmd } => match cmd {
            JobsCmd::List           => println!("[stub] GET {}/api/v1/jobs", cli.backend),
            JobsCmd::Show { id }    => println!("[stub] GET {}/api/v1/jobs/{id}", cli.backend),
            JobsCmd::Submit { spec } => println!("[stub] POST {}/api/v1/jobs (spec={spec})", cli.backend),
            JobsCmd::Cancel{ id }   => println!("[stub] POST {}/api/v1/jobs/{id}/cancel", cli.backend),
        },
    }
    Ok(())
}
