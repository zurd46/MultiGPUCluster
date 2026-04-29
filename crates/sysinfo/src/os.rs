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
    let device_name = detect_device_name();

    Ok(pb::OsInfo { family, version, kernel, arch, device_name })
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

pub fn hostname() -> String {
    // Cross-platform syscall via the `sysinfo` crate (it knows about
    // gethostname / GetComputerName / SCDynamicStoreCopyLocalHostName).
    // Fallback to env vars only as a last resort.
    System::host_name()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn detect_device_name() -> String {
    // macOS distinguishes the friendly *Computer Name* (e.g. "Daniel's
    // MacBook Pro") from the hostname (".local"-suffixed, no spaces).
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    // systemd-aware Linux distros expose a "pretty" name via hostnamectl.
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = std::process::Command::new("hostnamectl").arg("--pretty").output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    // Windows + everywhere else: no separate label, hostname IS the device name.
    hostname()
}
