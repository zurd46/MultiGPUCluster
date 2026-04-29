//! Streams a GGUF file from Hugging Face into the worker's `data-dir/models/`
//! directory.
//!
//! The Hub serves files at:
//!     https://huggingface.co/{repo}/resolve/main/{file}
//!
//! Public repos work without an Authorization header; gated/private ones need
//! `Authorization: Bearer <hf_token>` (a HF access token). We support both:
//! when `token` is empty we send no auth header.
//!
//! The download is atomic: we write to `<dest>.partial`, then rename. A
//! crash mid-download leaves the .partial behind, which the next attempt
//! overwrites — never a half-truncated `.gguf` that llama-server might try
//! to load.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub struct DownloadRequest<'a> {
    /// HuggingFace repo id, e.g. "bartowski/Llama-3.2-1B-Instruct-GGUF".
    pub repo: &'a str,
    /// File inside the repo, e.g. "Llama-3.2-1B-Instruct-Q4_K_M.gguf".
    pub file: &'a str,
    /// What to save it as on disk (relative to `models_dir`). Usually the
    /// same as `file`, but the admin can override (e.g. to keep two quants
    /// of the same model side-by-side).
    pub local_filename: &'a str,
    /// HF access token. Empty string means "no auth header".
    pub token: &'a str,
}

/// Returns the absolute path of the downloaded file. If a file with the same
/// name already exists and is non-empty, we treat it as cached and return
/// immediately — admins who want to force a re-download can delete the file.
pub async fn fetch(req: DownloadRequest<'_>, models_dir: &Path) -> Result<PathBuf> {
    if req.repo.is_empty() {
        return Err(anyhow!("hf_repo must not be empty"));
    }
    if req.file.is_empty() {
        return Err(anyhow!("hf_file must not be empty"));
    }
    fs::create_dir_all(models_dir)
        .await
        .with_context(|| format!("creating models dir {}", models_dir.display()))?;

    let dest = models_dir.join(req.local_filename);
    if let Ok(meta) = fs::metadata(&dest).await {
        if meta.is_file() && meta.len() > 0 {
            tracing::info!(path = %dest.display(), "model already cached locally; skipping download");
            return Ok(dest);
        }
    }
    let partial = dest.with_extension("partial");
    // Clean up any leftover from a previous interrupted run.
    let _ = fs::remove_file(&partial).await;

    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        req.repo, req.file
    );
    tracing::info!(%url, dest = %dest.display(), "starting HF download");

    let client = reqwest::Client::builder()
        // Big GGUFs can take a while on slow links — use no read timeout, but
        // bound the connect time so a wedged DNS doesn't hang us forever.
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut req_builder = client.get(&url);
    if !req.token.is_empty() {
        req_builder = req_builder.bearer_auth(req.token);
    }
    let resp = req_builder
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "huggingface returned {} for {}",
            resp.status(),
            url
        ));
    }

    let mut file = fs::File::create(&partial)
        .await
        .with_context(|| format!("creating {}", partial.display()))?;
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.with_context(|| "reading hf response chunk")?;
        file.write_all(&bytes).await.with_context(|| "writing to partial file")?;
    }
    file.flush().await.ok();
    drop(file);

    fs::rename(&partial, &dest)
        .await
        .with_context(|| format!("rename {} → {}", partial.display(), dest.display()))?;

    tracing::info!(path = %dest.display(), "model downloaded");
    Ok(dest)
}
