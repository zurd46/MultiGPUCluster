pub mod gpu;
pub mod os;
pub mod network;
pub mod fingerprint;
pub mod inventory;

#[cfg(target_os = "macos")]
mod gpu_metal;

use gpucluster_proto::node as pb;

pub fn collect() -> anyhow::Result<pb::NodeInfo> {
    let os_info = os::detect()?;
    let gpus = gpu::detect()?;
    let cpu_mem = os::cpu_mem();
    let network = network::detect();
    let hw_fingerprint = fingerprint::compute()?;
    let hostname = os::hostname();

    Ok(pb::NodeInfo {
        node_id: String::new(),
        hostname,
        display_name: String::new(),
        hw_fingerprint,
        owner_user_id: String::new(),
        tags: Vec::new(),
        os: Some(os_info),
        gpus,
        network: Some(network),
        cpu_mem: Some(cpu_mem),
        geo: None,
        // A worker that just collected fresh inventory is by definition
        // online — no other status is reachable from `collect()`. The
        // shutdown handler overrides this to `Draining` for the final
        // heartbeat; everything else relies on this default flowing through
        // the JSON wire format and being respected by the coordinator.
        status: pb::NodeStatus::Online as i32,
        first_seen: 0,
        last_heartbeat: 0,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        client_cert_sha: String::new(),
    })
}
