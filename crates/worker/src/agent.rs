use crate::{config::WorkerConfig, identity, heartbeat, rpc_backend};
use anyhow::Result;

pub async fn run(cfg: WorkerConfig) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir).ok();

    let node_id = identity::load_or_create_node_id(&cfg.data_dir)?;
    tracing::info!(%node_id, "node identity loaded");

    let info = gpucluster_sysinfo::collect()?;
    tracing::info!(
        gpus = info.gpus.len(),
        os = ?info.os.as_ref().map(|o| (&o.family, &o.version)),
        "collected sysinfo"
    );

    // Launch the matching ggml RPC backend (CUDA on Linux, Metal on macOS).
    // Failure here is non-fatal: a host with no GPU still enrolls so it can
    // appear in the dashboard, but stays ineligible for inference work.
    let backend = rpc_backend::RpcBackend::from_inventory(&info.gpus);
    let _rpc = match rpc_backend::RpcServer::spawn(backend, 50052) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "rpc-server-ext failed to start; node will run inference-ineligible");
            None
        }
    };

    heartbeat::run_loop(cfg.coordinator_url.clone(), node_id).await;
    Ok(())
}
