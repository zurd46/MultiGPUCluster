use anyhow::Result;
use std::time::Duration;

use crate::{docker, preflight, state};

pub async fn install() -> Result<()> {
    let pf = preflight::check()?;
    println!("docker:     {}", pf.docker_present);
    println!("nvidia-smi: {}", pf.nvidia_smi_present);
    println!("WSL:        {}", pf.is_wsl);
    println!("driver:     {:?}", pf.driver_version);

    if !pf.docker_present {
        anyhow::bail!("docker not present — install Docker Desktop (Win) or docker engine (Linux)");
    }

    #[cfg(unix)]
    write_systemd_unit()?;

    #[cfg(windows)]
    register_windows_service()?;

    println!("service installed. run `gpucluster-agent enroll --backend ... --token ...` next.");
    Ok(())
}

pub async fn uninstall(purge: bool) -> Result<()> {
    docker::stop_worker()?;
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["disable", "--now", "gpucluster-agent"]).status();
        let _ = std::fs::remove_file("/etc/systemd/system/gpucluster-agent.service");
    }
    if purge {
        let _ = std::fs::remove_dir_all(state::data_dir());
    }
    Ok(())
}

pub async fn status() -> Result<()> {
    let pf = preflight::check()?;
    let identity = state::load_identity()?;
    println!("== preflight ==");
    println!("  docker:        {}", pf.docker_present);
    println!("  nvidia-smi:    {}", pf.nvidia_smi_present);
    println!("  driver:        {:?}", pf.driver_version);
    println!("== identity ==");
    match identity {
        Some(i) => {
            println!("  node_id:       {}", i.node_id);
            println!("  coordinator:   {}", i.coordinator_endpoint);
            println!("  enrolled:      yes");
        }
        None => println!("  enrolled:      no"),
    }
    Ok(())
}

pub async fn run_loop() -> Result<()> {
    let identity = state::load_identity()?
        .ok_or_else(|| anyhow::anyhow!("not enrolled — run `gpucluster-agent enroll` first"))?;

    let pf = preflight::check()?;
    let image = docker::pick_image_tag_for_driver(pf.driver_version.as_deref());
    tracing::info!(%image, "selected worker image for driver");

    let _ = docker::pull_image(image);
    let env = [
        ("COORDINATOR_URL", identity.coordinator_endpoint.as_str()),
        ("NODE_ID",         identity.node_id.as_str()),
    ];
    let data_dir = state::data_dir().to_string_lossy().into_owned();
    docker::run_worker(image, &data_dir, &env)?;

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        // TODO: heartbeat to backend, watchdog of container, pull updates
        tracing::debug!("agent tick");
    }
}

#[cfg(unix)]
fn write_systemd_unit() -> Result<()> {
    const UNIT: &str = "[Unit]
Description=GPU Cluster Agent
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
ExecStart=/usr/local/bin/gpucluster-agent run
Restart=always
RestartSec=5
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
";
    let path = "/etc/systemd/system/gpucluster-agent.service";
    if let Err(e) = std::fs::write(path, UNIT) {
        tracing::warn!(error=%e, "failed to write systemd unit (need root)");
    }
    Ok(())
}

#[cfg(windows)]
fn register_windows_service() -> Result<()> {
    tracing::warn!("Windows Service registration: implement via `windows-service` crate in production");
    Ok(())
}
