// SPDX-License-Identifier: GPL-3.0-or-later
//! On-disk layout for downloaded ASR models.
//!
//! Files live under `$XDG_DATA_HOME/flowmux/asr/models/<id>.bin`. The
//! store is responsible for path resolution, partial-download
//! placeholders, and listing currently-installed models. The downloader
//! writes into a `<id>.bin.partial` next to the final name and renames
//! it after the SHA-256 check passes — that rename is atomic on every
//! POSIX filesystem flowmux supports, so a half-finished download cannot
//! be mistaken for a verified model file on the next launch.

use crate::catalog::{ModelEntry, ModelId};
use std::path::{Path, PathBuf};

/// Storage root. Built once at startup. Cloning is cheap — every field
/// is a `PathBuf`.
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    /// `$XDG_DATA_HOME/flowmux/asr/models` on Linux. Falls back to a
    /// best-effort `~/.local/share/...` when `XDG_DATA_HOME` is unset —
    /// matches the convention used by the rest of flowmux.
    pub fn xdg_default() -> Option<Self> {
        let data = dirs::data_dir()?;
        Some(Self {
            root: data.join("flowmux").join("asr").join("models"),
        })
    }

    /// Construct a store rooted under an explicit directory. Used by
    /// tests to keep all writes inside a `tempfile::tempdir()`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Final path of a verified model. The downloader renames a
    /// `.partial` here only after the SHA-256 check passes.
    pub fn model_path(&self, entry: &ModelEntry) -> PathBuf {
        self.root.join(entry.filename())
    }

    /// In-flight download target.
    pub fn partial_path(&self, entry: &ModelEntry) -> PathBuf {
        let mut name = entry.filename();
        name.push_str(".partial");
        self.root.join(name)
    }

    /// True if the verified file exists. Does *not* re-verify the
    /// SHA-256 here — the downloader has done that already; subsequent
    /// reads pay only filesystem cost.
    pub fn is_installed(&self, entry: &ModelEntry) -> bool {
        self.model_path(entry).exists()
    }

    /// Ensure the parent directory exists. Safe to call repeatedly.
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)
    }

    /// Drop the verified model and any partial. Used by the "Remove
    /// model" affordance in the options dialog.
    pub fn remove(&self, entry: &ModelEntry) -> std::io::Result<()> {
        let final_path = self.model_path(entry);
        let partial = self.partial_path(entry);
        if final_path.exists() {
            std::fs::remove_file(&final_path)?;
        }
        if partial.exists() {
            std::fs::remove_file(&partial)?;
        }
        Ok(())
    }

    /// Total bytes used by every verified model file under the root.
    /// Used to surface a "총 사용량" line in the options dialog.
    pub fn disk_usage(&self) -> std::io::Result<u64> {
        let Ok(read_dir) = std::fs::read_dir(&self.root) else {
            return Ok(0);
        };
        let mut total = 0;
        for ent in read_dir.flatten() {
            if let Ok(md) = ent.metadata() {
                if md.is_file() {
                    total += md.len();
                }
            }
        }
        Ok(total)
    }

    /// Returns ids whose `.bin` exists on disk. Order is unspecified.
    pub fn installed_ids(&self) -> Vec<ModelId> {
        let Ok(read_dir) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(stem) = name.strip_suffix(".bin") {
                out.push(ModelId::from(stem));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::recommended_default;
    use tempfile::TempDir;

    fn tmp() -> (TempDir, ModelStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().join("models"));
        store.ensure_dir().unwrap();
        (dir, store)
    }

    #[test]
    fn ensure_dir_creates_the_root_recursively() {
        let (_d, store) = tmp();
        assert!(store.root().is_dir());
    }

    #[test]
    fn partial_path_appends_partial_suffix_to_final_path() {
        let (_d, store) = tmp();
        let entry = recommended_default();
        let final_path = store.model_path(&entry);
        let partial = store.partial_path(&entry);
        let expected_partial = format!("{}.partial", final_path.to_string_lossy());
        assert_eq!(partial.to_string_lossy(), expected_partial);
        assert!(partial.to_string_lossy().ends_with(".bin.partial"));
    }

    #[test]
    fn is_installed_reflects_filesystem_state() {
        let (_d, store) = tmp();
        let entry = recommended_default();
        assert!(!store.is_installed(&entry));
        std::fs::write(store.model_path(&entry), b"fake").unwrap();
        assert!(store.is_installed(&entry));
    }

    #[test]
    fn remove_clears_verified_and_partial_files() {
        let (_d, store) = tmp();
        let entry = recommended_default();
        std::fs::write(store.model_path(&entry), b"a").unwrap();
        std::fs::write(store.partial_path(&entry), b"b").unwrap();
        store.remove(&entry).unwrap();
        assert!(!store.is_installed(&entry));
        assert!(!store.partial_path(&entry).exists());
    }

    #[test]
    fn installed_ids_lists_only_bin_files() {
        let (_d, store) = tmp();
        let entry = recommended_default();
        std::fs::write(store.model_path(&entry), b"x").unwrap();
        std::fs::write(store.root().join("README.md"), b"docs").unwrap();
        std::fs::write(store.partial_path(&entry), b"y").unwrap();
        let ids = store.installed_ids();
        assert_eq!(ids, vec![entry.id]);
    }

    #[test]
    fn disk_usage_sums_files_in_root() {
        let (_d, store) = tmp();
        std::fs::write(store.root().join("a.bin"), vec![0u8; 1024]).unwrap();
        std::fs::write(store.root().join("b.bin"), vec![0u8; 256]).unwrap();
        assert_eq!(store.disk_usage().unwrap(), 1024 + 256);
    }
}
