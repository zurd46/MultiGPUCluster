use crate::{config::WorkerConfig, identity, heartbeat};
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

    heartbeat::run_loop(cfg.coordinator_url.clone(), node_id).await;
    Ok(())
}
