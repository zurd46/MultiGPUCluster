use anyhow::Result;
use std::time::Duration;

use crate::{docker, native, preflight, state};

pub async fn install() -> Result<()> {
    let pf = preflight::check()?;
    println!("os:              {}", os_label(&pf));
    println!("docker:          {}", pf.docker_present);
    println!("nvidia-smi:      {}", pf.nvidia_smi_present);
    println!("apple-silicon:   {}", pf.is_apple_silicon);
    println!("WSL:             {}", pf.is_wsl);
    println!("driver:          {:?}", pf.driver_version);

    if pf.native_worker_required() {
        // macOS path: native binary, no Docker needed. We do require the
        // worker binary itself to be on PATH (shipped in the same .pkg).
        if !native::worker_binary_present() {
            anyhow::bail!(
                "native worker binary not found — install the macOS package \
                 (it ships gpucluster-worker alongside gpucluster-agent)"
            );
        }
        if !pf.is_apple_silicon {
            anyhow::bail!(
                "Intel macs are not supported as workers — Metal-accelerated \
                 inference requires Apple Silicon (M1 or newer)"
            );
        }
    } else if !pf.docker_present {
        anyhow::bail!("docker not present — install Docker Desktop (Win) or docker engine (Linux)");
    }

    #[cfg(target_os = "linux")]
    write_systemd_unit()?;

    #[cfg(target_os = "macos")]
    write_launchd_plist()?;

    #[cfg(windows)]
    register_windows_service()?;

    println!("service installed. run `gpucluster-agent enroll --backend ... --token ...` next.");
    Ok(())
}

pub async fn uninstall(purge: bool) -> Result<()> {
    let pf = preflight::check()?;
    if pf.native_worker_required() {
        let _ = native::stop_worker();
    } else {
        let _ = docker::stop_worker();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["disable", "--now", "gpucluster-agent"]).status();
        let _ = std::fs::remove_file("/etc/systemd/system/gpucluster-agent.service");
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", "system", LAUNCHD_PLIST_PATH]).status();
        let _ = std::fs::remove_file(LAUNCHD_PLIST_PATH);
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
    println!("  os:            {}", os_label(&pf));
    println!("  docker:        {}", pf.docker_present);
    println!("  nvidia-smi:    {}", pf.nvidia_smi_present);
    println!("  apple-silicon: {}", pf.is_apple_silicon);
    println!("  driver:        {:?}", pf.driver_version);
    if let Some(cpu) = &pf.cpu_brand {
        println!("  cpu:           {cpu}");
    }
    println!("== identity ==");
    match identity {
        Some(i) => {
            println!("  node_id:       {}", i.node_id);
            println!("  coordinator:   {}", i.coordinator_endpoint);
            println!("  enrolled:      yes");
        }
        None => println!("  enrolled:      no"),
    }

    // Full inventory — same block the worker prints on start and the same
    // shape the gateway sees via /cluster/nodes/report. One canonical view.
    match gpucluster_sysinfo::collect() {
        Ok(info) => {
            println!();
            print!("{}", gpucluster_sysinfo::inventory::format_human(&info));
        }
        Err(e) => println!("inventory: <unavailable> ({e})"),
    }
    Ok(())
}

pub async fn run_loop() -> Result<()> {
    let identity = state::load_identity()?
        .ok_or_else(|| anyhow::anyhow!("not enrolled — run `gpucluster-agent enroll` first"))?;

    let pf = preflight::check()?;
    let data_dir = state::data_dir().to_string_lossy().into_owned();
    let env: [(&str, &str); 2] = [
        ("COORDINATOR_URL", identity.coordinator_endpoint.as_str()),
        ("NODE_ID",         identity.node_id.as_str()),
    ];

    if pf.native_worker_required() {
        tracing::info!("starting worker natively (macOS / Apple Silicon)");
        native::run_worker(&data_dir, &env)?;
    } else {
        let image = docker::pick_image_tag_for_driver(pf.driver_version.as_deref());
        tracing::info!(%image, "selected worker image for driver");
        let _ = docker::pull_image(image);
        docker::run_worker(image, &data_dir, &env)?;
    }

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        // TODO: heartbeat to backend, watchdog of container/process, pull updates
        tracing::debug!("agent tick");
    }
}

fn os_label(pf: &preflight::Report) -> &'static str {
    if pf.is_macos { "macos" }
    else if pf.is_windows { "windows" }
    else if pf.is_wsl { "linux (WSL)" }
    else { "linux" }
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "macos")]
const LAUNCHD_PLIST_PATH: &str = "/Library/LaunchDaemons/com.gpucluster.agent.plist";

#[cfg(target_os = "macos")]
fn write_launchd_plist() -> Result<()> {
    // KeepAlive + ThrottleInterval gives us the same "respawn forever, but
    // not in a tight loop" semantics as Restart=always / RestartSec=5 on
    // systemd. RunAtLoad starts the service at boot.
    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.gpucluster.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/gpucluster-agent</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>5</integer>
    <key>StandardOutPath</key>
    <string>/Library/Logs/gpucluster/agent.log</string>
    <key>StandardErrorPath</key>
    <string>/Library/Logs/gpucluster/agent.err.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info,gpucluster_agent=debug</string>
    </dict>
</dict>
</plist>
"#;

    std::fs::create_dir_all("/Library/Logs/gpucluster").ok();

    if let Err(e) = std::fs::write(LAUNCHD_PLIST_PATH, PLIST) {
        tracing::warn!(error=%e, "failed to write launchd plist (need sudo)");
        return Ok(());
    }
    let _ = std::process::Command::new("launchctl")
        .args(["bootstrap", "system", LAUNCHD_PLIST_PATH])
        .status();
    Ok(())
}

#[cfg(windows)]
fn register_windows_service() -> Result<()> {
    tracing::warn!("Windows Service registration: implement via `windows-service` crate in production");
    Ok(())
}
