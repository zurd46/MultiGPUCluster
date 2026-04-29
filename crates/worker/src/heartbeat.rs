use std::time::Duration;
use tokio::time::interval;

pub async fn run_loop(coordinator_url: String, node_id: String) {
    let client = reqwest::Client::new();
    let mut tick = interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let url = format!("{coordinator_url}/health");
        match client.get(&url).send().await {
            Ok(r) => tracing::debug!(node = %node_id, status = %r.status(), "heartbeat (stub)"),
            Err(e) => tracing::warn!(error = %e, "heartbeat failed"),
        }
    }
}
