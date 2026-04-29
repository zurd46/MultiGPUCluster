//! Wire-format JSON view of `pb::NodeInfo` and friends.
//!
//! The proto-generated `pb::NodeInfo` doesn't derive serde because (a) prost
//! doesn't, and (b) we explicitly want the JSON wire shape to be decoupled
//! from the protobuf field tags — proto evolves with `optional` markers and
//! reordered fields, the JSON shape stays stable for the dashboard and DB.
//!
//! This module defines a parallel `NodeInfoView` (and sub-views) with serde
//! derives that mirror exactly what `gpucluster_sysinfo::inventory::to_json`
//! emits and what `coordinator::server::parse_node_info` consumes. With From
//! impls on both ends, we get:
//!
//!   pb::NodeInfo  ──Into──▶  NodeInfoView  ──serde_json──▶  wire JSON
//!                                ▲                              │
//!                                └────────serde_json───────────┘
//!                                ◀──From──── NodeInfoView ◀─── coordinator
//!
//! Result: the 110-line hand-rolled parser in coordinator collapses to one
//! `serde_json::from_value::<NodeInfoView>(...)?.into()`. Adding a field
//! means editing one struct in this file, not three locations across the
//! workspace.

use gpucluster_proto::node as pb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeInfoView {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub hw_fingerprint: String,
    #[serde(default)]
    pub agent_version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// String form of `pb::NodeStatus` — `"online"`, `"draining"`, etc.
    /// Optional on read so older workers (which never sent this field) keep
    /// working; absent → defaults to `Online` in `From`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatusView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsInfoView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_mem: Option<CpuMemInfoView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkInfoView>,
    #[serde(default)]
    pub gpus: Vec<GpuInfoView>,

    // Sidecar fields the worker tacks on when reporting — they aren't part
    // of pb::NodeInfo but the coordinator pulls them straight off the JSON
    // body. Keep them here so the View round-trips losslessly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatusView {
    #[default]
    Unspecified,
    PendingApproval,
    Online,
    Degraded,
    Draining,
    Offline,
    Quarantined,
    Revoked,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsInfoView {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub device_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuMemInfoView {
    #[serde(default)]
    pub cpu_model: String,
    #[serde(default)]
    pub cpu_cores: u32,
    #[serde(default)]
    pub cpu_threads: u32,
    #[serde(default)]
    pub ram_total_bytes: u64,
    #[serde(default)]
    pub ram_free_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkInfoView {
    #[serde(default)]
    pub public_ip_v4: String,
    #[serde(default)]
    pub public_ip_v6: String,
    #[serde(default)]
    pub local_ips: Vec<String>,
    #[serde(default)]
    pub primary_iface: String,
    #[serde(default)]
    pub link_speed_mbps: u32,
    #[serde(default)]
    pub wg_ip: String,
    #[serde(default)]
    pub wg_pubkey_sha: String,
    #[serde(default)]
    pub wg_listen_port: u32,
    #[serde(default)]
    pub rtt_to_gateway_ms: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuInfoView {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub backend: BackendView,
    #[serde(default)]
    pub compute_cap_major: u32,
    #[serde(default)]
    pub compute_cap_minor: u32,
    #[serde(default)]
    pub vram_total_bytes: u64,
    #[serde(default)]
    pub vram_free_bytes: u64,
    #[serde(default)]
    pub unified_memory: bool,
    #[serde(default)]
    pub gpu_core_count: u32,
    #[serde(default)]
    pub metal_family: String,
    #[serde(default)]
    pub driver_version: String,
    #[serde(default)]
    pub cuda_version: String,
    #[serde(default)]
    pub vbios_version: String,
    #[serde(default)]
    pub power_limit_w: u32,
    #[serde(default)]
    pub nvlink_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<GpuCapabilityProfileView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuCapabilityProfileView {
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub compute_cap_major: u32,
    #[serde(default)]
    pub compute_cap_minor: u32,
    #[serde(default)]
    pub vram_bytes: u64,
    #[serde(default)]
    pub mem_bandwidth_gbs: f32,
    #[serde(default)]
    pub supports_fp16: bool,
    #[serde(default)]
    pub supports_bf16: bool,
    #[serde(default)]
    pub supports_fp8: bool,
    #[serde(default)]
    pub supports_fp4: bool,
    #[serde(default)]
    pub supports_int8_tc: bool,
    #[serde(default)]
    pub backend: BackendView,
    #[serde(default)]
    pub unified_memory: bool,
}

/// JSON-friendly wire form of `pb::GpuBackend`. Existing deployments emit
/// these as lowercase strings (see `inventory::backend_label`); the prost
/// enum is `i32` underneath so we use a separate type to round-trip cleanly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendView {
    #[default]
    Unspecified,
    Cuda,
    Metal,
    Rocm,
    Vulkan,
}

// ---------- conversions ----------

impl From<NodeStatusView> for pb::NodeStatus {
    fn from(v: NodeStatusView) -> Self {
        match v {
            NodeStatusView::Unspecified => pb::NodeStatus::Unspecified,
            NodeStatusView::PendingApproval => pb::NodeStatus::PendingApproval,
            NodeStatusView::Online => pb::NodeStatus::Online,
            NodeStatusView::Degraded => pb::NodeStatus::Degraded,
            NodeStatusView::Draining => pb::NodeStatus::Draining,
            NodeStatusView::Offline => pb::NodeStatus::Offline,
            NodeStatusView::Quarantined => pb::NodeStatus::Quarantined,
            NodeStatusView::Revoked => pb::NodeStatus::Revoked,
        }
    }
}
impl From<pb::NodeStatus> for NodeStatusView {
    fn from(v: pb::NodeStatus) -> Self {
        match v {
            pb::NodeStatus::Unspecified => NodeStatusView::Unspecified,
            pb::NodeStatus::PendingApproval => NodeStatusView::PendingApproval,
            pb::NodeStatus::Online => NodeStatusView::Online,
            pb::NodeStatus::Degraded => NodeStatusView::Degraded,
            pb::NodeStatus::Draining => NodeStatusView::Draining,
            pb::NodeStatus::Offline => NodeStatusView::Offline,
            pb::NodeStatus::Quarantined => NodeStatusView::Quarantined,
            pb::NodeStatus::Revoked => NodeStatusView::Revoked,
        }
    }
}

impl From<BackendView> for pb::GpuBackend {
    fn from(v: BackendView) -> Self {
        match v {
            BackendView::Unspecified => pb::GpuBackend::Unspecified,
            BackendView::Cuda => pb::GpuBackend::Cuda,
            BackendView::Metal => pb::GpuBackend::Metal,
            BackendView::Rocm => pb::GpuBackend::Rocm,
            BackendView::Vulkan => pb::GpuBackend::Vulkan,
        }
    }
}
impl From<pb::GpuBackend> for BackendView {
    fn from(v: pb::GpuBackend) -> Self {
        match v {
            pb::GpuBackend::Unspecified => BackendView::Unspecified,
            pb::GpuBackend::Cuda => BackendView::Cuda,
            pb::GpuBackend::Metal => BackendView::Metal,
            pb::GpuBackend::Rocm => BackendView::Rocm,
            pb::GpuBackend::Vulkan => BackendView::Vulkan,
        }
    }
}

impl From<NodeInfoView> for pb::NodeInfo {
    fn from(v: NodeInfoView) -> Self {
        let status = v
            .status
            .map(pb::NodeStatus::from)
            .unwrap_or(pb::NodeStatus::Online);
        pb::NodeInfo {
            node_id: v.node_id,
            hostname: v.hostname,
            display_name: v.display_name,
            hw_fingerprint: v.hw_fingerprint,
            owner_user_id: String::new(),
            tags: v.tags,
            os: v.os.map(Into::into),
            gpus: v.gpus.into_iter().map(Into::into).collect(),
            network: v.network.map(Into::into),
            cpu_mem: v.cpu_mem.map(Into::into),
            geo: None,
            status: status as i32,
            first_seen: 0,
            last_heartbeat: 0,
            agent_version: v.agent_version,
            client_cert_sha: String::new(),
        }
    }
}

impl From<&pb::NodeInfo> for NodeInfoView {
    fn from(v: &pb::NodeInfo) -> Self {
        let status_enum =
            pb::NodeStatus::try_from(v.status).unwrap_or(pb::NodeStatus::Unspecified);
        Self {
            node_id: v.node_id.clone(),
            hostname: v.hostname.clone(),
            display_name: v.display_name.clone(),
            hw_fingerprint: v.hw_fingerprint.clone(),
            agent_version: v.agent_version.clone(),
            tags: v.tags.clone(),
            status: Some(status_enum.into()),
            os: v.os.as_ref().map(Into::into),
            cpu_mem: v.cpu_mem.as_ref().map(Into::into),
            network: v.network.as_ref().map(Into::into),
            gpus: v.gpus.iter().map(Into::into).collect(),
            inference_endpoint: None,
            control_endpoint: None,
            current_model: None,
        }
    }
}

impl From<OsInfoView> for pb::OsInfo {
    fn from(v: OsInfoView) -> Self {
        pb::OsInfo {
            family: v.family,
            version: v.version,
            kernel: v.kernel,
            arch: v.arch,
            device_name: v.device_name,
        }
    }
}
impl From<&pb::OsInfo> for OsInfoView {
    fn from(v: &pb::OsInfo) -> Self {
        Self {
            family: v.family.clone(),
            version: v.version.clone(),
            kernel: v.kernel.clone(),
            arch: v.arch.clone(),
            device_name: v.device_name.clone(),
        }
    }
}

impl From<CpuMemInfoView> for pb::CpuMemInfo {
    fn from(v: CpuMemInfoView) -> Self {
        pb::CpuMemInfo {
            cpu_model: v.cpu_model,
            cpu_cores: v.cpu_cores,
            cpu_threads: v.cpu_threads,
            ram_total_bytes: v.ram_total_bytes,
            ram_free_bytes: v.ram_free_bytes,
        }
    }
}
impl From<&pb::CpuMemInfo> for CpuMemInfoView {
    fn from(v: &pb::CpuMemInfo) -> Self {
        Self {
            cpu_model: v.cpu_model.clone(),
            cpu_cores: v.cpu_cores,
            cpu_threads: v.cpu_threads,
            ram_total_bytes: v.ram_total_bytes,
            ram_free_bytes: v.ram_free_bytes,
        }
    }
}

impl From<NetworkInfoView> for pb::NetworkInfo {
    fn from(v: NetworkInfoView) -> Self {
        pb::NetworkInfo {
            public_ip_v4: v.public_ip_v4,
            public_ip_v6: v.public_ip_v6,
            asn: String::new(),
            isp: String::new(),
            country_code: String::new(),
            city: String::new(),
            public_ip_is_dynamic: false,
            public_ip_changed_at: 0,
            local_ips: v.local_ips,
            primary_iface: v.primary_iface,
            link_speed_mbps: v.link_speed_mbps,
            wg_ip: v.wg_ip,
            wg_pubkey_sha: v.wg_pubkey_sha,
            wg_listen_port: v.wg_listen_port,
            rtt_to_gateway_ms: v.rtt_to_gateway_ms,
        }
    }
}
impl From<&pb::NetworkInfo> for NetworkInfoView {
    fn from(v: &pb::NetworkInfo) -> Self {
        Self {
            public_ip_v4: v.public_ip_v4.clone(),
            public_ip_v6: v.public_ip_v6.clone(),
            local_ips: v.local_ips.clone(),
            primary_iface: v.primary_iface.clone(),
            link_speed_mbps: v.link_speed_mbps,
            wg_ip: v.wg_ip.clone(),
            wg_pubkey_sha: v.wg_pubkey_sha.clone(),
            wg_listen_port: v.wg_listen_port,
            rtt_to_gateway_ms: v.rtt_to_gateway_ms,
        }
    }
}

impl From<GpuInfoView> for pb::GpuInfo {
    fn from(v: GpuInfoView) -> Self {
        let backend_i = pb::GpuBackend::from(v.backend) as i32;
        pb::GpuInfo {
            index: v.index,
            uuid: v.uuid,
            name: v.name,
            architecture: v.architecture,
            compute_cap_major: v.compute_cap_major,
            compute_cap_minor: v.compute_cap_minor,
            vram_total_bytes: v.vram_total_bytes,
            vram_free_bytes: v.vram_free_bytes,
            pci_bus_id: String::new(),
            driver_version: v.driver_version,
            cuda_version: v.cuda_version,
            vbios_version: v.vbios_version,
            power_limit_w: v.power_limit_w,
            nvlink_present: v.nvlink_present,
            capability: v.capability.map(Into::into),
            backend: backend_i,
            unified_memory: v.unified_memory,
            gpu_core_count: v.gpu_core_count,
            metal_family: v.metal_family,
        }
    }
}
impl From<&pb::GpuInfo> for GpuInfoView {
    fn from(v: &pb::GpuInfo) -> Self {
        let backend_enum =
            pb::GpuBackend::try_from(v.backend).unwrap_or(pb::GpuBackend::Unspecified);
        Self {
            index: v.index,
            uuid: v.uuid.clone(),
            name: v.name.clone(),
            architecture: v.architecture.clone(),
            backend: backend_enum.into(),
            compute_cap_major: v.compute_cap_major,
            compute_cap_minor: v.compute_cap_minor,
            vram_total_bytes: v.vram_total_bytes,
            vram_free_bytes: v.vram_free_bytes,
            unified_memory: v.unified_memory,
            gpu_core_count: v.gpu_core_count,
            metal_family: v.metal_family.clone(),
            driver_version: v.driver_version.clone(),
            cuda_version: v.cuda_version.clone(),
            vbios_version: v.vbios_version.clone(),
            power_limit_w: v.power_limit_w,
            nvlink_present: v.nvlink_present,
            capability: v.capability.as_ref().map(Into::into),
        }
    }
}

impl From<GpuCapabilityProfileView> for pb::GpuCapabilityProfile {
    fn from(v: GpuCapabilityProfileView) -> Self {
        let backend_i = pb::GpuBackend::from(v.backend) as i32;
        pb::GpuCapabilityProfile {
            architecture: v.architecture,
            compute_cap_major: v.compute_cap_major,
            compute_cap_minor: v.compute_cap_minor,
            vram_bytes: v.vram_bytes,
            mem_bandwidth_gbs: v.mem_bandwidth_gbs,
            supports_fp16: v.supports_fp16,
            supports_bf16: v.supports_bf16,
            supports_fp8: v.supports_fp8,
            supports_fp4: v.supports_fp4,
            supports_int8_tc: v.supports_int8_tc,
            benchmark: None,
            backend: backend_i,
            unified_memory: v.unified_memory,
        }
    }
}
impl From<&pb::GpuCapabilityProfile> for GpuCapabilityProfileView {
    fn from(v: &pb::GpuCapabilityProfile) -> Self {
        let backend_enum =
            pb::GpuBackend::try_from(v.backend).unwrap_or(pb::GpuBackend::Unspecified);
        Self {
            architecture: v.architecture.clone(),
            compute_cap_major: v.compute_cap_major,
            compute_cap_minor: v.compute_cap_minor,
            vram_bytes: v.vram_bytes,
            mem_bandwidth_gbs: v.mem_bandwidth_gbs,
            supports_fp16: v.supports_fp16,
            supports_bf16: v.supports_bf16,
            supports_fp8: v.supports_fp8,
            supports_fp4: v.supports_fp4,
            supports_int8_tc: v.supports_int8_tc,
            backend: backend_enum.into(),
            unified_memory: v.unified_memory,
        }
    }
}
