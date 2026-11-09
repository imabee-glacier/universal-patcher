use crate::model::Manifest;
use anyhow::{Context, Result, ensure};
use futures_util::TryStreamExt;
use std::path::Path;
use url::Url;

pub struct VerifiedRelease {
    pub(crate) manifest: Manifest,
    pub(crate) digest: String,
}
impl VerifiedRelease {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
/// Fetches the signed target bytes after full TUF verification (signatures, expiry, rollback).
pub async fn load_bytes(
    root: &Path,
    metadata: Url,
    targets: Url,
    name: &str,
    state: &Path,
) -> Result<Vec<u8>> {
    for u in [&metadata, &targets] {
        ensure!(
            u.scheme() == "https"
                || u.scheme() == "file"
                || (u.scheme() == "http"
                    && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
            "Feed requires HTTPS (loopback permitted for local tests)"
        );
    }
    std::fs::create_dir_all(state)?;
    let root = tokio::fs::read(root).await?;
    let repo = tough::RepositoryLoader::new(&root, metadata, targets)
        .datastore(state)
        .load()
        .await
        .context("Release metadata verification failed")?;
    let mut stream = repo
        .read_target(&tough::TargetName::new(name)?)
        .await?
        .context("Release is not a signed target")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        ensure!(
            bytes.len() + chunk.len() <= 32 * 1024 * 1024,
            "Manifest exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn load(
    root: &Path,
    metadata: Url,
    targets: Url,
    name: &str,
    state: &Path,
) -> Result<VerifiedRelease> {
    let bytes = load_bytes(root, metadata, targets, name, state).await?;
    let manifest = Manifest::parse(&bytes)?;
    Ok(VerifiedRelease {
        manifest,
        digest: blake3::hash(&bytes).to_hex().to_string(),
    })
}
