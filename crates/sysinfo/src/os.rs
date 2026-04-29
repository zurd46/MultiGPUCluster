use anyhow::Result;
use gpucluster_proto::node as pb;
use sysinfo::System;

pub fn detect() -> Result<pb::OsInfo> {
    let family = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    }
    .to_string();

    let version = System::os_version().unwrap_or_default();
    let kernel = System::kernel_version().unwrap_or_default();
    let arch = std::env::consts::ARCH.to_string();

    Ok(pb::OsInfo { family, version, kernel, arch })
}

pub fn cpu_mem() -> pb::CpuMemInfo {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_model = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default();
    let cpu_threads = sys.cpus().len() as u32;
    let cpu_cores = sys.physical_core_count().map(|v| v as u32).unwrap_or(cpu_threads);

    pb::CpuMemInfo {
        cpu_model,
        cpu_cores,
        cpu_threads,
        ram_total_bytes: sys.total_memory(),
        ram_free_bytes:  sys.available_memory(),
    }
}
