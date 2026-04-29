use crate::{config::WorkerConfig, heartbeat, identity, inference_server, rpc_backend};
use anyhow::Result;
use gpucluster_sysinfo::inventory;

pub async fn run(cfg: WorkerConfig) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir).ok();

    let node_id = identity::load_or_create_node_id(&cfg.data_dir)?;
    tracing::info!(%node_id, "node identity loaded");

    let mut info = gpucluster_sysinfo::collect()?;
    info.node_id = node_id.clone();
    if let Some(name) = cfg.display_name.clone() {
        info.display_name = name;
    }

    // Full inventory to local logs — operators see exactly what the gateway is
    // about to be told. Same string is rendered by `gpucluster-agent status`.
    println!("{}", inventory::format_human(&info));

    // Launch the matching ggml RPC backend (CUDA on Linux, Metal on macOS).
    // Failure here is non-fatal: a host with no GPU still enrolls so it can
    // appear in the dashboard, but stays ineligible for inference work.
    let backend = rpc_backend::RpcBackend::from_inventory(&info.gpus);
    let _rpc = match rpc_backend::RpcServer::spawn(backend, 50052) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "rpc-server-ext failed to start; node stays inference-ineligible at the RPC layer");
            None
        }
    };

    // Phase 2 single-worker inference: when MODEL_PATH is set, also spawn
    // `llama-server` on port 50053. The coordinator-eligible view advertises
    // this endpoint so the cluster's openai-api can forward chat requests.
    let inference = match inference_server::InferenceServer::try_spawn(
        inference_server::DEFAULT_INFERENCE_PORT,
    ) {
        Ok(Some(srv)) => {
            tracing::info!(port = srv.port, "inference endpoint live");
            Some(srv)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "llama-server failed to start; inference disabled on this node");
            None
        }
    };
    let inference_endpoint = inference.as_ref().map(|s| {
        // The worker doesn't know its own externally-routable IP yet; the
        // coordinator captures it from the incoming TCP socket. So we only
        // advertise the port — the dispatcher pairs it with the public/wg IP
        // it already has from /nodes/eligible.
        format!(":{}", s.port)
    });
    let _inference = inference;

    heartbeat::run_loop(cfg.coordinator_url.clone(), info, inference_endpoint).await;
    Ok(())
}
