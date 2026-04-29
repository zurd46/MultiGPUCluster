use anyhow::{Context, Result};
use std::path::Path;

pub fn load_or_create_node_id(data_dir: &str) -> Result<String> {
    let path = Path::new(data_dir).join("node.id");
    if path.exists() {
        let s = std::fs::read_to_string(&path).context("read node.id")?;
        return Ok(s.trim().to_string());
    }
    std::fs::create_dir_all(data_dir).context("create data dir")?;
    let id = uuid::Uuid::now_v7().to_string();
    std::fs::write(&path, &id).context("write node.id")?;
    Ok(id)
}
