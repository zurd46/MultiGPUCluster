use anyhow::Result;
use gpucluster_proto::node as pb;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn detect() -> Result<Vec<pb::GpuInfo>> {
    use nvml_wrapper::Nvml;

    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "NVML init failed; reporting empty GPU list");
            return Ok(Vec::new());
        }
    };

    let mut out = Vec::new();
    let count = nvml.device_count().unwrap_or(0);
    let driver_version = nvml.sys_driver_version().unwrap_or_default();
    let cuda_version = nvml.sys_cuda_driver_version().ok().map(|v| {
        let major = v / 1000;
        let minor = (v % 1000) / 10;
        format!("{major}.{minor}")
    }).unwrap_or_default();

    for i in 0..count {
        let dev = match nvml.device_by_index(i) {
            Ok(d) => d,
            Err(e) => { tracing::warn!(idx=i, error=%e, "device_by_index failed"); continue; }
        };

        let mem = dev.memory_info().ok();
        let cc = dev.cuda_compute_capability().ok();
        let pci = dev.pci_info().ok();
        let arch = cc.map(|c| classify_arch(c.major, c.minor)).unwrap_or_default();

        out.push(pb::GpuInfo {
            index: i,
            uuid: dev.uuid().unwrap_or_default(),
            name: dev.name().unwrap_or_default(),
            architecture: arch.clone(),
            compute_cap_major: cc.map(|c| c.major as u32).unwrap_or(0),
            compute_cap_minor: cc.map(|c| c.minor as u32).unwrap_or(0),
            vram_total_bytes: mem.as_ref().map(|m| m.total).unwrap_or(0),
            vram_free_bytes:  mem.as_ref().map(|m| m.free).unwrap_or(0),
            pci_bus_id: pci.map(|p| p.bus_id).unwrap_or_default(),
            driver_version: driver_version.clone(),
            cuda_version: cuda_version.clone(),
            vbios_version: dev.vbios_version().unwrap_or_default(),
            power_limit_w: dev.power_management_limit().unwrap_or(0) / 1000,
            nvlink_present: false,
            capability: Some(build_capability(&arch, cc, mem.as_ref().map(|m| m.total).unwrap_or(0))),
        });
    }

    Ok(out)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn detect() -> Result<Vec<pb::GpuInfo>> {
    Ok(Vec::new())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn classify_arch(major: i32, minor: i32) -> String {
    match (major, minor) {
        (12, _) => "Blackwell".into(),
        (9, _)  => "Hopper".into(),
        (8, 9)  => "Ada".into(),
        (8, _)  => "Ampere".into(),
        (7, 5)  => "Turing".into(),
        (7, _)  => "Volta".into(),
        (6, _)  => "Pascal".into(),
        _       => format!("sm_{major}{minor}"),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn build_capability(
    arch: &str,
    cc: Option<nvml_wrapper::struct_wrappers::device::CudaComputeCapability>,
    vram: u64,
) -> pb::GpuCapabilityProfile {
    let major = cc.map(|c| c.major as u32).unwrap_or(0);
    let minor = cc.map(|c| c.minor as u32).unwrap_or(0);

    let supports_bf16 = major >= 8;
    let supports_fp16 = major >= 6;
    let supports_fp8  = (major == 8 && minor >= 9) || major >= 9;
    let supports_fp4  = major >= 12;
    let supports_int8_tc = major >= 7;

    pb::GpuCapabilityProfile {
        architecture: arch.to_string(),
        compute_cap_major: major,
        compute_cap_minor: minor,
        vram_bytes: vram,
        mem_bandwidth_gbs: 0.0,
        supports_fp16,
        supports_bf16,
        supports_fp8,
        supports_fp4,
        supports_int8_tc,
        benchmark: None,
    }
}
