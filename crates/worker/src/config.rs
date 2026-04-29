#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub coordinator_url: String,
    pub data_dir: String,
    pub display_name: Option<String>,
}
