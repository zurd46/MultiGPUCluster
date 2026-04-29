use anyhow::Result;
use std::process::Command;

pub fn check() -> Result<Report> {
    let mut r = Report::default();

    r.docker_present = which("docker");
    r.nvidia_smi_present = which("nvidia-smi");
    r.is_windows = cfg!(windows);
    r.is_macos   = cfg!(target_os = "macos");
    r.is_wsl = std::fs::read_to_string("/proc/version")
        .map(|s| s.to_lowercase().contains("microsoft"))
        .unwrap_or(false);

    if let Ok(out) = Command::new("nvidia-smi")
        .arg("--query-gpu=driver_version")
        .arg("--format=csv,noheader")
        .output()
    {
        if out.status.success() {
            r.driver_version = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }

    // Apple Silicon detection — `sysctl hw.optional.arm64` returns "1" on
    // Apple Silicon and 0 / nothing on Intel macs. We treat AS as the only
    // macOS hardware that can act as an inference worker.
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl").args(["-n", "hw.optional.arm64"]).output() {
            if out.status.success() {
                r.is_apple_silicon = String::from_utf8_lossy(&out.stdout).trim() == "1";
            }
        }
        if let Ok(out) = Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output() {
            if out.status.success() {
                r.cpu_brand = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
        }
    }

    Ok(r)
}

#[derive(Debug, Default)]
pub struct Report {
    pub docker_present: bool,
    pub nvidia_smi_present: bool,
    pub is_windows: bool,
    pub is_macos: bool,
    pub is_wsl: bool,
    pub is_apple_silicon: bool,
    pub driver_version: Option<String>,
    pub cpu_brand: Option<String>,
}

impl Report {
    /// Whether this host should run the worker as a native binary (no Docker).
    /// macOS is the only platform that takes this path today, because Metal
    /// can't be passed into a Linux container.
    pub fn native_worker_required(&self) -> bool {
        self.is_macos
    }
}

fn which(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
