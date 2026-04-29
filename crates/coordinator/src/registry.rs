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
    /// Worker-advertised port (e.g. `":50054"`) for the control plane —
    /// receives `load_model` and friends. Same IP-pairing dance as
    /// `inference_endpoint`. `None` for legacy workers.
    pub control_endpoint: Option<String>,
    /// id of the model currently loaded on this worker, as reported by its
    /// heartbeat. Empty / None means "no model loaded yet" (workers that
    /// never received a load_model RPC, or that started with `MODEL_PATH=`).
    pub current_model: Option<String>,
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
        control_endpoint: Option<String>,
        current_model: Option<String>,
    ) {
        let entry = NodeEntry {
            status: pb::NodeStatus::try_from(info.status).unwrap_or(pb::NodeStatus::Unspecified),
            info: info.clone(),
            last_heartbeat: Utc::now(),
            current_public_ip: public_ip,
            inference_endpoint,
            control_endpoint,
            current_model,
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

    /// Returns `Some(node_id)` if there is *another* entry with the same
    /// `hw_fingerprint` whose last heartbeat is fresher than `stale_after`.
    /// Used to reject a second worker process trying to register from the
    /// same physical machine under a new identity (typical cause: someone
    /// started the worker with a different `--data-dir`, getting a fresh
    /// `node.id` file and reporting as a "new" node).
    ///
    /// Empty `hw_fingerprint` is never considered a conflict — Phase 1 dev
    /// builds without sysinfo populated would otherwise lock each other out.
    pub fn find_active_with_hw_fingerprint(
        &self,
        hw_fingerprint: &str,
        exclude_node_id: &str,
        stale_after: chrono::Duration,
    ) -> Option<String> {
        if hw_fingerprint.is_empty() {
            return None;
        }
        let now = Utc::now();
        for entry in self.inner.iter() {
            let e = entry.value();
            if e.info.node_id == exclude_node_id {
                continue;
            }
            if e.info.hw_fingerprint != hw_fingerprint {
                continue;
            }
            if now.signed_duration_since(e.last_heartbeat) <= stale_after {
                return Some(e.info.node_id.clone());
            }
        }
        None
    }
}
