pub mod placement;
pub mod compute_group;
pub mod precision;

use gpucluster_proto::node as pb;

pub struct PlacementRequest {
    pub model_total_layers: u32,
    pub model_bytes: u64,
    pub require_arch_min: Option<String>,
    pub prefer_homogeneous: bool,
    /// Numeric precision the model needs. The placer drops GPUs whose
    /// capability profile says they can't honour it — this is what lets the
    /// scheduler safely mix CUDA and Metal in one pipeline (both must agree
    /// on the wire format the RPC stages exchange).
    pub required_precision: precision::Precision,
}

impl Default for PlacementRequest {
    fn default() -> Self {
        Self {
            model_total_layers: 0,
            model_bytes: 0,
            require_arch_min: None,
            prefer_homogeneous: false,
            required_precision: precision::Precision::Bf16,
        }
    }
}

pub struct StageAssignment {
    pub node_id: String,
    pub gpu_index: u32,
    pub layer_start: u32,
    pub layer_end: u32,
    /// Backend that the worker has to start its RPC server in (CUDA vs Metal).
    pub backend: pb::GpuBackend,
}

pub trait Placer {
    fn plan(&self, req: &PlacementRequest, nodes: &[pb::NodeInfo])
        -> anyhow::Result<Vec<StageAssignment>>;
}
