use anyhow::{anyhow, Result};
use std::process::Command;

pub fn pull_image(image: &str) -> Result<()> {
    let status = Command::new("docker").args(["pull", image]).status()?;
    if !status.success() { return Err(anyhow!("docker pull {} failed", image)); }
    Ok(())
}

pub fn run_worker(image: &str, data_dir: &str, env: &[(&str, &str)]) -> Result<()> {
    let mut args: Vec<String> = vec![
        "run".into(), "--rm".into(), "-d".into(),
        "--name".into(), "gpucluster-worker".into(),
        "--gpus".into(), "all".into(),
        "--network".into(), "host".into(),
        "--ipc".into(), "host".into(),
        "--cap-add".into(), "NET_ADMIN".into(),
        "--ulimit".into(), "memlock=-1".into(),
        "--ulimit".into(), "stack=67108864".into(),
        "-v".into(), format!("{data_dir}:/var/lib/gpucluster"),
    ];
    for (k, v) in env {
        args.push("-e".into());
        args.push(format!("{k}={v}"));
    }
    args.push(image.into());

    let status = Command::new("docker").args(&args).status()?;
    if !status.success() { return Err(anyhow!("docker run failed")); }
    Ok(())
}

pub fn stop_worker() -> Result<()> {
    let _ = Command::new("docker").args(["stop", "gpucluster-worker"]).status();
    Ok(())
}

pub fn pick_image_tag_for_driver(driver_version: Option<&str>) -> &'static str {
    match driver_version.and_then(parse_major) {
        Some(m) if m >= 555 => "ghcr.io/dzurmuehle/gpucluster-worker:0.1.0-cuda12.8",
        Some(m) if m >= 535 => "ghcr.io/dzurmuehle/gpucluster-worker:0.1.0-cuda12.4",
        Some(m) if m >= 520 => "ghcr.io/dzurmuehle/gpucluster-worker:0.1.0-cuda11.8",
        _                   => "ghcr.io/dzurmuehle/gpucluster-worker:0.1.0-cuda12.4",
    }
}

fn parse_major(s: &str) -> Option<u32> {
    s.split('.').next().and_then(|m| m.trim().parse().ok())
}
