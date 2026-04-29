use gpucluster_proto::node as pb;
use std::collections::HashMap;

pub struct ComputeGroup {
    pub key: String,
    pub members: Vec<String>,
}

pub fn partition(nodes: &[pb::NodeInfo]) -> Vec<ComputeGroup> {
    let mut buckets: HashMap<String, Vec<String>> = HashMap::new();
    for n in nodes {
        if let Some(g) = n.gpus.first() {
            let key = format!("{}-{}.{}", g.architecture, g.compute_cap_major, g.compute_cap_minor);
            buckets.entry(key).or_default().push(n.node_id.clone());
        }
    }
    buckets.into_iter().map(|(key, members)| ComputeGroup { key, members }).collect()
}
