use chrono::{DateTime, Utc};
use dashmap::DashMap;
use gpucluster_proto::node as pb;
use std::sync::Arc;

#[derive(Clone)]
pub struct NodeEntry {
    pub info: pb::NodeInfo,
    pub last_heartbeat: DateTime<Utc>,
    pub current_public_ip: Option<String>,
    pub status: pb::NodeStatus,
    /// Worker-advertised port (e.g. `":50053"`) where its `llama-server` is
    /// listening. The dispatcher pairs this with `current_public_ip` to build
    /// a full URL: `http://<public_ip>:50053`. `None` for nodes that don't
    /// have a model loaded.
    pub inference_endpoint: Option<String>,
}

#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<DashMap<String, NodeEntry>>,
}

impl Registry {
    pub fn new() -> Self { Self::default() }

    pub fn upsert(
        &self,
        info: pb::NodeInfo,
        public_ip: Option<String>,
        inference_endpoint: Option<String>,
    ) {
        let entry = NodeEntry {
            status: pb::NodeStatus::try_from(info.status).unwrap_or(pb::NodeStatus::Unspecified),
            info: info.clone(),
            last_heartbeat: Utc::now(),
            current_public_ip: public_ip,
            inference_endpoint,
        };
        self.inner.insert(info.node_id, entry);
    }

    pub fn touch(&self, node_id: &str) {
        if let Some(mut e) = self.inner.get_mut(node_id) {
            e.last_heartbeat = Utc::now();
        }
    }

    pub fn list(&self) -> Vec<NodeEntry> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<NodeEntry> {
        self.inner.get(id).map(|e| e.value().clone())
    }

    pub fn count(&self) -> usize { self.inner.len() }
}
