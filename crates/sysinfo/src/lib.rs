pub mod gpu;
pub mod os;
pub mod network;
pub mod fingerprint;

use gpucluster_proto::node as pb;

pub fn collect() -> anyhow::Result<pb::NodeInfo> {
    let os = os::detect()?;
    let gpus = gpu::detect()?;
    let cpu_mem = os::cpu_mem();
    let network = network::detect();
    let hw_fingerprint = fingerprint::compute()?;

    Ok(pb::NodeInfo {
        node_id: String::new(),
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_default(),
        display_name: String::new(),
        hw_fingerprint,
        owner_user_id: String::new(),
        tags: Vec::new(),
        os: Some(os),
        gpus,
        network: Some(network),
        cpu_mem: Some(cpu_mem),
        geo: None,
        status: pb::NodeStatus::Unspecified as i32,
        first_seen: 0,
        last_heartbeat: 0,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        client_cert_sha: String::new(),
    })
}

mod hostname {
    pub fn get() -> Option<std::ffi::OsString> {
        std::env::var_os("HOSTNAME").or_else(|| std::env::var_os("COMPUTERNAME"))
    }
}
