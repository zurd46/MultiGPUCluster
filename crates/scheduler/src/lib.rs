pub mod placement;
pub mod compute_group;

use gpucluster_proto::node as pb;

pub struct PlacementRequest {
    pub model_total_layers: u32,
    pub model_bytes: u64,
    pub require_arch_min: Option<String>,
    pub prefer_homogeneous: bool,
}

pub struct StageAssignment {
    pub node_id: String,
    pub gpu_index: u32,
    pub layer_start: u32,
    pub layer_end: u32,
}

pub trait Placer {
    fn plan(&self, req: &PlacementRequest, nodes: &[pb::NodeInfo])
        -> anyhow::Result<Vec<StageAssignment>>;
}
