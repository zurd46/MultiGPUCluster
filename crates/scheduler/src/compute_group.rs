use gpucluster_proto::node as pb;
use std::collections::HashMap;

pub struct ComputeGroup {
    /// Stable, human-readable bucket key. NVIDIA GPUs are keyed by CUDA
    /// compute capability ("cuda-Ampere-8.0"); Apple Silicon by chip family
    /// ("metal-Apple-M3-Max"); ROCm/Vulkan reserve their own prefixes for
    /// future use. The backend prefix is what prevents the scheduler from
    /// trying to lump a 4090 and an M3 Max into the same TP group.
    pub key: String,
    pub members: Vec<String>,
}

pub fn partition(nodes: &[pb::NodeInfo]) -> Vec<ComputeGroup> {
    let mut buckets: HashMap<String, Vec<String>> = HashMap::new();
    for n in nodes {
        if let Some(g) = n.gpus.first() {
            buckets.entry(group_key(g)).or_default().push(n.node_id.clone());
        }
    }
    buckets.into_iter().map(|(key, members)| ComputeGroup { key, members }).collect()
}

/// Returns the compute-group bucket for a GPU. TP (tensor parallelism) is only
/// allowed *within* a group; PP (pipeline parallelism) is allowed across.
pub fn group_key(g: &pb::GpuInfo) -> String {
    let backend = pb::GpuBackend::try_from(g.backend).unwrap_or(pb::GpuBackend::Unspecified);
    match backend {
        pb::GpuBackend::Metal => {
            // Compute caps are 0 for Apple GPUs; lump all M3 Max together
            // regardless of binned core count — same instruction set, same
            // unified-memory model, TP-safe.
            format!("metal-{}", g.architecture)
        }
        pb::GpuBackend::Rocm => format!("rocm-{}", g.architecture),
        pb::GpuBackend::Vulkan => format!("vulkan-{}", g.architecture),
        // Default = CUDA. Preserves the original "{arch}-{cc.major}.{cc.minor}"
        // shape but namespaced so a future backend can't collide.
        _ => format!(
            "cuda-{}-{}.{}",
            g.architecture, g.compute_cap_major, g.compute_cap_minor
        ),
    }
}
