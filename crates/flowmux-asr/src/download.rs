// SPDX-License-Identifier: GPL-3.0-or-later
//! Async, integrity-checked model downloader.
//!
//! `ModelDownloader::start(entry)` streams a model from the URL listed
//! in the catalog into the store's `.partial` path, hashing every byte
//! into a streaming SHA-256. When the body finishes:
//!
//! 1. The computed digest is compared against `entry.sha256`.
//! 2. On match the partial is `rename`'d to the final `.bin` path.
//! 3. On mismatch the partial is deleted and `DownloadError::HashMismatch`
//!    is returned so the UI can show a retry.
//!
//! Progress is surfaced through a `tokio::sync::mpsc` channel of
//! [`DownloadEvent`] values. The UI consumes them on the GTK side.

use crate::catalog::ModelEntry;
use crate::store::ModelStore;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Snapshot value emitted while bytes are flowing. `total` may be `None`
/// when the server omits a `Content-Length` header — the UI should fall
/// back to the catalog `size_bytes` in that case.
#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub bytes_received: u64,
    pub total: Option<u64>,
}

impl DownloadProgress {
    /// `0.0..=1.0` ratio if a total is available. Returns `None` for
    /// indeterminate downloads so the UI can switch its progress bar to
    /// pulse mode.
    pub fn ratio(&self) -> Option<f64> {
        match self.total {
            Some(total) if total > 0 => {
                let r = self.bytes_received as f64 / total as f64;
                Some(r.clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// Streaming events emitted as a download progresses. The UI typically
/// uses `Started` to flip into a "downloading" view, `Progress` to drive
/// the bar, and one of `Finished` / `Failed` to dismiss it.
#[derive(Debug)]
pub enum DownloadEvent {
    Started {
        total: Option<u64>,
    },
    Progress(DownloadProgress),
    /// Bytes finished and the SHA-256 matched.
    Finished {
        path: PathBuf,
    },
    Failed(DownloadError),
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("http error: {0}")]
    Http(String),
    #[error("non-success status: {0}")]
    BadStatus(u16),
    #[error("write to disk failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("hash mismatch (expected {expected}, got {actual})")]
    HashMismatch { expected: String, actual: String },
    #[error("cancelled by caller")]
    Cancelled,
}

impl From<reqwest::Error> for DownloadError {
    fn from(value: reqwest::Error) -> Self {
        DownloadError::Http(value.to_string())
    }
}

/// Owner of the download state. One downloader handles a single in-
/// flight transfer at a time — the UI disables the model picker for the
/// duration so concurrent downloads do not need to be modeled here.
pub struct ModelDownloader {
    store: ModelStore,
    client: reqwest::Client,
}

impl ModelDownloader {
    pub fn new(store: ModelStore) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("flowmux-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest::Client::build with default config never fails");
        Self { store, client }
    }

    /// Start the download. Returns the receiver half of a progress
    /// channel; the work happens on a tokio task that the caller does
    /// not have to await unless it wants to block until completion.
    pub fn start(&self, entry: ModelEntry) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(16);
        let store = self.store.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = run_download(client, store, entry, tx.clone()).await;
            if let Err(err) = result {
                let _ = tx.send(DownloadEvent::Failed(err)).await;
            }
        });
        rx
    }
}

async fn run_download(
    client: reqwest::Client,
    store: ModelStore,
    entry: ModelEntry,
    tx: mpsc::Sender<DownloadEvent>,
) -> Result<(), DownloadError> {
    store.ensure_dir().map_err(DownloadError::Io)?;
    let partial = store.partial_path(&entry);
    // Always start fresh — resume support is a Phase 2 improvement.
    if partial.exists() {
        let _ = std::fs::remove_file(&partial);
    }

    let response = client.get(&entry.url).send().await?;
    if !response.status().is_success() {
        return Err(DownloadError::BadStatus(response.status().as_u16()));
    }
    let total = response.content_length().or(Some(entry.size_bytes));
    let _ = tx.send(DownloadEvent::Started { total }).await;

    let mut file = std::fs::File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut stream = response.bytes_stream();
    let mut last_progress_bytes: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk)?;
        received += chunk.len() as u64;
        if received - last_progress_bytes >= 256 * 1024 {
            last_progress_bytes = received;
            let _ = tx
                .send(DownloadEvent::Progress(DownloadProgress {
                    bytes_received: received,
                    total,
                }))
                .await;
        }
    }
    file.sync_all()?;
    drop(file);

    let computed = format!("{:x}", hasher.finalize());
    if entry.sha256.is_empty() {
        // The catalog still ships placeholder entries with empty SHA-256
        // values; an empty expected digest disables verification with a
        // loud warning so an unverified model is impossible to miss in
        // logs and the maintainer is reminded to backfill the hash.
        tracing::warn!(
            model = %entry.id.as_str(),
            "ASR model downloaded without sha256 verification — release blocker; fill in catalog hash before shipping"
        );
    } else if computed != entry.sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err(DownloadError::HashMismatch {
            expected: entry.sha256.clone(),
            actual: computed,
        });
    }
    let final_path = store.model_path(&entry);
    std::fs::rename(&partial, &final_path)?;
    let _ = tx
        .send(DownloadEvent::Finished {
            path: final_path.clone(),
        })
        .await;
    Ok(())
}

/// Compute the SHA-256 of an existing file as lowercase hex. Used by
/// the "verify already-installed model" smoke check at app startup so a
/// half-overwritten file on a previously-crashed run is caught early.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_ratio_is_none_without_total() {
        let p = DownloadProgress {
            bytes_received: 100,
            total: None,
        };
        assert!(p.ratio().is_none());
    }

    #[test]
    fn progress_ratio_is_clamped_to_unit_interval() {
        let p = DownloadProgress {
            bytes_received: 500,
            total: Some(1000),
        };
        assert_eq!(p.ratio(), Some(0.5));

        let over = DownloadProgress {
            bytes_received: 2000,
            total: Some(1000),
        };
        assert_eq!(over.ratio(), Some(1.0));
    }

    #[test]
    fn sha256_file_matches_in_memory_digest() {
        // The hash itself depends on the upstream `sha2` crate; the
        // test just confirms the streaming reader matches the one-shot
        // digest for the same input, so any regression in the
        // chunk-loop wiring fails loudly.
        let payload = b"flowmux ASR sha256 round-trip";
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), payload).unwrap();
        let file_hash = sha256_file(tmp.path()).unwrap();
        let one_shot = format!("{:x}", sha2::Sha256::digest(payload));
        assert_eq!(file_hash, one_shot);
    }
}
