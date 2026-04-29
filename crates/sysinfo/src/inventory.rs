//! Cross-platform inventory rendering.
//!
//! Two outputs from the same `pb::NodeInfo` snapshot:
//!   - `format_human()`  → multi-line string for logs, `gpucluster-agent status`,
//!                         and the enroll-confirmation block
//!   - `to_json()`       → `serde_json::Value` shipped to the gateway
//!                         (enrollment payload + heartbeat /nodes/report)
//!
//! Both views share the same field naming so the dashboard, the CLI and the
//! local logs can't drift. If a field exists in one, it exists in the other.

use gpucluster_proto::node as pb;
use serde_json::{json, Value};

/// Render a complete inventory block. Platform-agnostic — works for
/// CUDA hosts, Apple Silicon hosts, and GPU-less hosts that just enrolled
/// for visibility.
pub fn format_human(info: &pb::NodeInfo) -> String {
    let mut s = String::new();
    s.push_str("== Host ==\n");
    if let Some(os) = &info.os {
        let device = if os.device_name.is_empty() { &info.hostname } else { &os.device_name };
        s.push_str(&format!("  device:        {}\n", device));
        s.push_str(&format!("  hostname:      {}\n", info.hostname));
        s.push_str(&format!("  os:            {} {}\n", os.family, os.version));
        if !os.kernel.is_empty() {
            s.push_str(&format!("  kernel:        {}\n", os.kernel));
        }
        s.push_str(&format!("  arch:          {}\n", os.arch));
    } else {
        s.push_str(&format!("  hostname:      {}\n", info.hostname));
    }
    if !info.hw_fingerprint.is_empty() {
        s.push_str(&format!("  fingerprint:   {}\n", info.hw_fingerprint));
    }
    s.push_str(&format!("  agent:         v{}\n", info.agent_version));

    if let Some(cm) = &info.cpu_mem {
        s.push_str("== CPU / RAM ==\n");
        s.push_str(&format!("  cpu:           {} ({}c / {}t)\n",
            cm.cpu_model, cm.cpu_cores, cm.cpu_threads));
        s.push_str(&format!("  ram:           {} (free {})\n",
            human_bytes(cm.ram_total_bytes), human_bytes(cm.ram_free_bytes)));
    }

    s.push_str(&format!("== GPUs ({}) ==\n", info.gpus.len()));
    if info.gpus.is_empty() {
        s.push_str("  (none — node will enroll but stay inference-ineligible)\n");
    } else {
        for g in &info.gpus {
            let backend = backend_label(g.backend);
            let arch_disp = if g.compute_cap_major > 0 || g.compute_cap_minor > 0 {
                format!("{} (sm_{}{})", g.architecture, g.compute_cap_major, g.compute_cap_minor)
            } else {
                g.architecture.clone()
            };
            s.push_str(&format!("  [{}] {}\n", g.index, g.name));
            s.push_str(&format!("       backend:    {backend}\n"));
            s.push_str(&format!("       arch:       {arch_disp}\n"));
            if g.unified_memory {
                s.push_str(&format!("       memory:     {} (unified)\n",
                    human_bytes(g.vram_total_bytes)));
            } else {
                s.push_str(&format!("       vram:       {} (free {})\n",
                    human_bytes(g.vram_total_bytes), human_bytes(g.vram_free_bytes)));
            }
            if g.gpu_core_count > 0 {
                s.push_str(&format!("       cores:      {} GPU cores\n", g.gpu_core_count));
            }
            if !g.driver_version.is_empty() {
                s.push_str(&format!("       driver:     {}\n", g.driver_version));
            }
            if !g.cuda_version.is_empty() {
                s.push_str(&format!("       cuda:       {}\n", g.cuda_version));
            }
            if !g.metal_family.is_empty() {
                s.push_str(&format!("       metal:      {}\n", g.metal_family));
            }
            if let Some(c) = &g.capability {
                let mut precs = Vec::new();
                if c.supports_fp16 { precs.push("FP16"); }
                if c.supports_bf16 { precs.push("BF16"); }
                if c.supports_fp8  { precs.push("FP8"); }
                if c.supports_fp4  { precs.push("FP4"); }
                if c.supports_int8_tc { precs.push("INT8-TC"); }
                if !precs.is_empty() {
                    s.push_str(&format!("       precision:  {}\n", precs.join(" · ")));
                }
            }
        }
    }

    if let Some(net) = &info.network {
        s.push_str("== Network ==\n");
        if !net.public_ip_v4.is_empty() {
            s.push_str(&format!("  public-v4:     {}\n", net.public_ip_v4));
        }
        if !net.local_ips.is_empty() {
            s.push_str(&format!("  local:         {}\n", net.local_ips.join(", ")));
        }
        if !net.wg_ip.is_empty() {
            s.push_str(&format!("  wg:            {} (rtt-gw {} ms)\n",
                net.wg_ip, net.rtt_to_gateway_ms));
        }
    }

    s
}

/// Wire format for upload to the gateway — used by enrollment and
/// `/cluster/nodes/report` heartbeats. Kept hand-written instead of letting
/// prost+serde derive it so the field names stay stable across proto evolutions
/// (the dashboard and DB schema can rely on these keys without a regen step).
///
/// Includes `status` (snake-case enum string) so a graceful-shutdown
/// heartbeat with `pb::NodeStatus::Draining` propagates through the
/// coordinator instead of getting silently overwritten to "online".
pub fn to_json(info: &pb::NodeInfo) -> Value {
    json!({
        "node_id":          info.node_id,
        "hostname":         info.hostname,
        "display_name":     info.display_name,
        "hw_fingerprint":   info.hw_fingerprint,
        "agent_version":    info.agent_version,
        "tags":             info.tags,
        "status":           status_label(info.status),
        "os":               info.os.as_ref().map(os_json).unwrap_or(Value::Null),
        "cpu_mem":          info.cpu_mem.as_ref().map(cpu_mem_json).unwrap_or(Value::Null),
        "network":          info.network.as_ref().map(net_json).unwrap_or(Value::Null),
        "gpus":             info.gpus.iter().map(gpu_json).collect::<Vec<_>>(),
    })
}

fn status_label(status: i32) -> &'static str {
    match pb::NodeStatus::try_from(status).unwrap_or(pb::NodeStatus::Unspecified) {
        pb::NodeStatus::Unspecified     => "unspecified",
        pb::NodeStatus::PendingApproval => "pending_approval",
        pb::NodeStatus::Online          => "online",
        pb::NodeStatus::Degraded        => "degraded",
        pb::NodeStatus::Draining        => "draining",
        pb::NodeStatus::Offline         => "offline",
        pb::NodeStatus::Quarantined     => "quarantined",
        pb::NodeStatus::Revoked         => "revoked",
    }
}

fn os_json(os: &pb::OsInfo) -> Value {
    json!({
        "family":      os.family,
        "version":     os.version,
        "kernel":      os.kernel,
        "arch":        os.arch,
        "device_name": os.device_name,
    })
}

fn cpu_mem_json(cm: &pb::CpuMemInfo) -> Value {
    json!({
        "cpu_model":       cm.cpu_model,
        "cpu_cores":       cm.cpu_cores,
        "cpu_threads":     cm.cpu_threads,
        "ram_total_bytes": cm.ram_total_bytes,
        "ram_free_bytes":  cm.ram_free_bytes,
    })
}

fn net_json(n: &pb::NetworkInfo) -> Value {
    json!({
        "public_ip_v4":   n.public_ip_v4,
        "public_ip_v6":   n.public_ip_v6,
        "local_ips":      n.local_ips,
        "primary_iface":  n.primary_iface,
        "link_speed_mbps": n.link_speed_mbps,
        "wg_ip":          n.wg_ip,
        "wg_pubkey_sha":  n.wg_pubkey_sha,
        "wg_listen_port": n.wg_listen_port,
        "rtt_to_gateway_ms": n.rtt_to_gateway_ms,
    })
}

fn gpu_json(g: &pb::GpuInfo) -> Value {
    json!({
        "index":             g.index,
        "uuid":              g.uuid,
        "name":              g.name,
        "architecture":      g.architecture,
        "backend":           backend_label(g.backend),
        "compute_cap_major": g.compute_cap_major,
        "compute_cap_minor": g.compute_cap_minor,
        "vram_total_bytes":  g.vram_total_bytes,
        "vram_free_bytes":   g.vram_free_bytes,
        "unified_memory":    g.unified_memory,
        "gpu_core_count":    g.gpu_core_count,
        "metal_family":      g.metal_family,
        "driver_version":    g.driver_version,
        "cuda_version":      g.cuda_version,
        "vbios_version":     g.vbios_version,
        "power_limit_w":     g.power_limit_w,
        "nvlink_present":    g.nvlink_present,
        "capability":        g.capability.as_ref().map(cap_json).unwrap_or(Value::Null),
    })
}

fn cap_json(c: &pb::GpuCapabilityProfile) -> Value {
    json!({
        "architecture":      c.architecture,
        "compute_cap_major": c.compute_cap_major,
        "compute_cap_minor": c.compute_cap_minor,
        "vram_bytes":        c.vram_bytes,
        "mem_bandwidth_gbs": c.mem_bandwidth_gbs,
        "supports_fp16":     c.supports_fp16,
        "supports_bf16":     c.supports_bf16,
        "supports_fp8":      c.supports_fp8,
        "supports_fp4":      c.supports_fp4,
        "supports_int8_tc":  c.supports_int8_tc,
        "backend":           backend_label(c.backend),
        "unified_memory":    c.unified_memory,
    })
}

pub fn backend_label(backend: i32) -> &'static str {
    match pb::GpuBackend::try_from(backend).unwrap_or(pb::GpuBackend::Unspecified) {
        pb::GpuBackend::Cuda    => "cuda",
        pb::GpuBackend::Metal   => "metal",
        pb::GpuBackend::Rocm    => "rocm",
        pb::GpuBackend::Vulkan  => "vulkan",
        pb::GpuBackend::Unspecified => "unspecified",
    }
}

fn human_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    if b >= TB { format!("{:.2} TB", b as f64 / TB as f64) }
    else if b >= GB { format!("{:.2} GB", b as f64 / GB as f64) }
    else if b >= MB { format!("{:.2} MB", b as f64 / MB as f64) }
    else if b >= KB { format!("{:.2} KB", b as f64 / KB as f64) }
    else            { format!("{} B", b) }
}
