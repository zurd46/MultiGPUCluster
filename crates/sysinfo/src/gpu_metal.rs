//! Apple Silicon GPU detection via `system_profiler` (ships with macOS, no extra deps).
//!
//! We deliberately avoid pulling in `metal-rs` / `objc2` to keep the worker a
//! lean Rust binary — `system_profiler SPDisplaysDataType -json` and `sysctl`
//! give us everything we need for inventory + scheduling. Live VRAM telemetry
//! (free / used) for unified-memory systems is reported via the `vm_stat`
//! / `host_statistics64` path in a follow-up; for now we report total RAM as
//! the budget, which matches Apple's recommended_max_working_set_size on
//! integrated GPUs to within a few percent.
//!
//! What we extract:
//!   - chip family (M1 / M2 / M3 / M4 + Pro/Max/Ultra suffix)
//!   - GPU core count
//!   - unified memory size (== system memory)
//!   - precision support derived from family
//!   - Metal feature-set family (Metal3 for M1/M2, Metal4 for M3+)
//!
//! Output is normalised into the same `GpuInfo` shape used for NVIDIA, with
//! `backend = METAL` and CUDA-specific fields zeroed.

use anyhow::{Context, Result};
use gpucluster_proto::node as pb;
use std::process::Command;

pub fn detect() -> Result<Vec<pb::GpuInfo>> {
    // Only Apple Silicon ships a unified-memory GPU; refuse early on Intel macs.
    if !is_apple_silicon() {
        tracing::info!("not Apple Silicon (Intel mac) — no Metal GPU reported");
        return Ok(Vec::new());
    }

    let raw = match run_system_profiler() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "system_profiler failed; reporting empty GPU list");
            return Ok(Vec::new());
        }
    };

    let parsed: SpReport = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "could not parse system_profiler JSON");
            return Ok(Vec::new());
        }
    };

    let total_ram = sysctl_u64("hw.memsize").unwrap_or(0);
    let chip_brand = sysctl_string("machdep.cpu.brand_string").unwrap_or_default();
    let os_build = sysctl_string("kern.osversion").unwrap_or_default();

    let mut out = Vec::new();
    for (i, dev) in parsed.SPDisplaysDataType.iter().enumerate() {
        // `_name` is the display device name. Apple Silicon iGPUs always carry
        // "Apple M…"; external eGPUs (rare on AS) we skip for now.
        let name = dev._name.clone().unwrap_or_default();
        if !name.starts_with("Apple M") {
            continue;
        }

        let family = parse_family(&name).unwrap_or_else(|| AppleFamily::generic(&name));
        let cores = dev
            .sppci_cores
            .as_ref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(family.default_cores());

        let metal_family = match family.generation {
            1 | 2 => "Metal3",
            3 | 4 => "Metal4",
            _     => "Metal",
        }.to_string();

        let cap = pb::GpuCapabilityProfile {
            architecture:       family.arch_string(),
            compute_cap_major:  0,
            compute_cap_minor:  0,
            vram_bytes:         total_ram,
            mem_bandwidth_gbs:  family.estimated_bandwidth_gbs(cores),
            supports_fp16:      true,
            // BF16 in hardware: M3 and newer (Metal 3.1+ exposes it). M1/M2
            // emulate via FP32, which we treat as "no" for scheduler purposes
            // — they fall back to FP16 anyway.
            supports_bf16:      family.generation >= 3,
            supports_fp8:       false,
            supports_fp4:       false,
            supports_int8_tc:   true,
            benchmark:          None,
            backend:            pb::GpuBackend::Metal as i32,
            unified_memory:     true,
        };

        out.push(pb::GpuInfo {
            index: i as u32,
            uuid: format!("metal-{}", chip_brand.replace(' ', "-")),
            name,
            architecture:       family.arch_string(),
            compute_cap_major:  0,
            compute_cap_minor:  0,
            vram_total_bytes:   total_ram,
            vram_free_bytes:    total_ram, // best-effort; refined later via vm_stat
            pci_bus_id:         String::new(),
            driver_version:     os_build.clone(),
            cuda_version:       String::new(),
            vbios_version:      String::new(),
            power_limit_w:      0,
            nvlink_present:     false,
            capability:         Some(cap),
            backend:            pb::GpuBackend::Metal as i32,
            unified_memory:     true,
            gpu_core_count:     cores,
            metal_family,
        });
    }

    Ok(out)
}

fn is_apple_silicon() -> bool {
    // hw.optional.arm64 == 1 on Apple Silicon, 0 (or missing) on Intel.
    sysctl_u64("hw.optional.arm64").unwrap_or(0) == 1
}

fn run_system_profiler() -> Result<String> {
    let out = Command::new("system_profiler")
        .args(["-json", "SPDisplaysDataType"])
        .output()
        .context("spawn system_profiler")?;
    if !out.status.success() {
        anyhow::bail!("system_profiler exited {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn sysctl_string(key: &str) -> Option<String> {
    let out = Command::new("sysctl").args(["-n", key]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn sysctl_u64(key: &str) -> Option<u64> {
    sysctl_string(key)?.parse().ok()
}

#[derive(serde::Deserialize)]
#[allow(non_snake_case)]
struct SpReport {
    SPDisplaysDataType: Vec<SpDisplay>,
}

#[derive(serde::Deserialize)]
struct SpDisplay {
    _name: Option<String>,
    sppci_cores: Option<String>,
}

struct AppleFamily {
    /// 1, 2, 3, 4 → M1, M2, M3, M4
    generation: u32,
    /// "", "Pro", "Max", "Ultra"
    variant: &'static str,
    /// raw label, e.g. "Apple M3 Max"
    label: String,
}

impl AppleFamily {
    fn generic(label: &str) -> Self {
        Self { generation: 0, variant: "", label: label.to_string() }
    }

    fn arch_string(&self) -> String {
        // Scheduler bucket key (see compute_group). We intentionally use a
        // human-friendly form: "Apple-M3-Max" — easy to grep, readable in
        // the dashboard, stable across releases.
        if self.variant.is_empty() {
            format!("Apple-M{}", self.generation)
        } else {
            format!("Apple-M{}-{}", self.generation, self.variant)
        }
    }

    fn default_cores(&self) -> u32 {
        // Used only if system_profiler doesn't report core count
        // (older macOS releases). Conservative defaults.
        match (self.generation, self.variant) {
            (1, "")      => 8,   (1, "Pro")   => 16,  (1, "Max")   => 32,  (1, "Ultra") => 64,
            (2, "")      => 10,  (2, "Pro")   => 19,  (2, "Max")   => 38,  (2, "Ultra") => 76,
            (3, "")      => 10,  (3, "Pro")   => 18,  (3, "Max")   => 40,  (3, "Ultra") => 80,
            (4, "")      => 10,  (4, "Pro")   => 20,  (4, "Max")   => 40,  (4, "Ultra") => 80,
            _            => 8,
        }
    }

    fn estimated_bandwidth_gbs(&self, _cores: u32) -> f32 {
        // Per Apple's published unified memory bandwidth (rounded). Used as a
        // placement hint only; replaced by measured bench score after join.
        match (self.generation, self.variant) {
            (1, "")      => 68.0,   (1, "Pro")   => 200.0,  (1, "Max")   => 400.0,  (1, "Ultra") => 800.0,
            (2, "")      => 100.0,  (2, "Pro")   => 200.0,  (2, "Max")   => 400.0,  (2, "Ultra") => 800.0,
            (3, "")      => 100.0,  (3, "Pro")   => 150.0,  (3, "Max")   => 300.0,  (3, "Ultra") => 800.0,
            (4, "")      => 120.0,  (4, "Pro")   => 273.0,  (4, "Max")   => 546.0,  (4, "Ultra") => 1092.0,
            _            => 100.0,
        }
    }
}

fn parse_family(name: &str) -> Option<AppleFamily> {
    // Names look like "Apple M1", "Apple M2 Pro", "Apple M3 Max", "Apple M4 Ultra".
    let rest = name.strip_prefix("Apple M")?;
    let mut parts = rest.split_whitespace();
    let gen_str = parts.next()?;
    let generation: u32 = gen_str.parse().ok()?;
    let variant = match parts.next() {
        Some("Pro")   => "Pro",
        Some("Max")   => "Max",
        Some("Ultra") => "Ultra",
        _             => "",
    };
    Some(AppleFamily { generation, variant, label: name.to_string() })
}
