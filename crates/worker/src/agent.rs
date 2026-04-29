use crate::{
    config::WorkerConfig, control_server, heartbeat, identity, inference_server, rpc_backend,
};
use anyhow::Result;
use gpucluster_sysinfo::inventory;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn run(cfg: WorkerConfig) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir).ok();
    // Pre-create the models subdir so the control server's first download
    // doesn't race the directory creation across tasks.
    std::fs::create_dir_all(format!("{}/models", cfg.data_dir.trim_end_matches('/'))).ok();

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

    // Single supervisor owns the llama-server lifecycle. The control server
    // can swap the model out at runtime; the heartbeat task reads the
    // current advertised endpoint + model id straight from the same struct.
    let sup = Arc::new(Mutex::new(inference_server::Supervisor::boot(
        inference_server::DEFAULT_INFERENCE_PORT,
    )));

    // Control plane — receives load_model RPCs from the coordinator.
    let _control = control_server::spawn(
        control_server::DEFAULT_CONTROL_PORT,
        sup.clone(),
        &cfg.data_dir,
    );
    let control_endpoint = control_server::endpoint_advertise(control_server::DEFAULT_CONTROL_PORT);

    heartbeat::run_loop(cfg.coordinator_url.clone(), info, sup, control_endpoint).await;
    Ok(())
}
