use anyhow::Result;
use std::process::Command;

pub fn check() -> Result<Report> {
    let mut r = Report::default();

    r.docker_present = which("docker");
    r.nvidia_smi_present = which("nvidia-smi");
    r.is_windows = cfg!(windows);
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

    Ok(r)
}

#[derive(Debug, Default)]
pub struct Report {
    pub docker_present: bool,
    pub nvidia_smi_present: bool,
    pub is_windows: bool,
    pub is_wsl: bool,
    pub driver_version: Option<String>,
}

fn which(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
