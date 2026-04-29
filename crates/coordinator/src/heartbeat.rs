use crate::registry::Registry;
use chrono::{Duration, Utc};
use gpucluster_proto::node as pb;
use std::time::Duration as StdDuration;
use tokio::time::interval;

pub async fn watchdog(reg: Registry) {
    let mut tick = interval(StdDuration::from_secs(5));
    loop {
        tick.tick().await;
        let now = Utc::now();
        let stale = Duration::seconds(60);
        for entry in reg.list() {
            if now.signed_duration_since(entry.last_heartbeat) > stale {
                tracing::warn!(node_id = %entry.info.node_id, "node marked offline");
                let mut info = entry.info.clone();
                info.status = pb::NodeStatus::Offline as i32;
                reg.upsert(info, entry.current_public_ip, entry.inference_endpoint);
            }
        }
    }
}
