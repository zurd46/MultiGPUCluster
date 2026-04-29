use crate::{Placer, PlacementRequest, StageAssignment};
use gpucluster_proto::node as pb;

pub struct GreedyVramWeighted;

impl Placer for GreedyVramWeighted {
    fn plan(
        &self,
        req: &PlacementRequest,
        nodes: &[pb::NodeInfo],
    ) -> anyhow::Result<Vec<StageAssignment>> {
        // 1. Flatten and filter by required precision so a Metal-only model can
        //    legitimately span an M3 Max + an RTX 4090, while an FP8 job skips
        //    everything pre-Ada and every Apple GPU.
        let mut gpus: Vec<(&pb::NodeInfo, &pb::GpuInfo)> = nodes
            .iter()
            .flat_map(|n| n.gpus.iter().map(move |g| (n, g)))
            .filter(|(_, g)| {
                g.capability
                    .as_ref()
                    .map_or(false, |c| req.required_precision.supported_by(c))
            })
            .collect();

        gpus.sort_by(|a, b| b.1.vram_free_bytes.cmp(&a.1.vram_free_bytes));

        let total_vram: u64 = gpus.iter().map(|(_, g)| g.vram_free_bytes).sum();
        if total_vram == 0 {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let mut cursor: u32 = 0;
        for (i, (n, g)) in gpus.iter().enumerate() {
            let share = (g.vram_free_bytes as f64) / (total_vram as f64);
            let mut layers = (share * req.model_total_layers as f64).round() as u32;
            if i == gpus.len() - 1 {
                layers = req.model_total_layers - cursor;
            }
            if layers == 0 { continue; }
            out.push(StageAssignment {
                node_id: n.node_id.clone(),
                gpu_index: g.index,
                layer_start: cursor,
                layer_end: cursor + layers,
                backend: pb::GpuBackend::try_from(g.backend)
                    .unwrap_or(pb::GpuBackend::Unspecified),
            });
            cursor += layers;
        }
        Ok(out)
    }
}
