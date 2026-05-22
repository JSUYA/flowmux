// SPDX-License-Identifier: GPL-3.0-or-later
//! Static catalog of supported Whisper models.
//!
//! Each [`ModelEntry`] is the source of truth for one downloadable model:
//! its stable `id`, the display label shown in the options dialog, the
//! Hugging Face URL it is fetched from, the SHA-256 of the expected
//! payload, the approximate size, and the supported language profile.
//!
//! Adding a new model is the only change required in the catalog — the
//! downloader, store, and UI all enumerate this list directly.

use serde::{Deserialize, Serialize};

/// Stable identifier persisted in `options.json` under
/// `asr.active_model_id`. The wire form mirrors the upstream
/// `ggerganov/whisper.cpp` filenames so newcomers can map them at a
/// glance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(pub String);

impl ModelId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Language coverage of a model. Used by the UI to decide whether a
/// language picker should be presented next to the model selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelLanguages {
    /// Whisper multilingual checkpoints support ~100 languages including
    /// Korean.
    Multilingual,
    /// English-only checkpoints. Smaller and a little faster but reject
    /// non-English audio.
    English,
}

/// Catalog row. Fields are intentionally `String` rather than `&'static
/// str` so callers can clone the entry into UI rows without dragging the
/// catalog lifetime around.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: ModelId,
    pub display: String,
    pub url: String,
    /// Lower-case hex SHA-256 of the expected payload. The downloader
    /// verifies and then atomically renames the file into place.
    pub sha256: String,
    pub size_bytes: u64,
    pub languages: ModelLanguages,
    /// Short human hint shown in the picker — "권장", "저사양", "최고 품질".
    pub recommendation: String,
}

impl ModelEntry {
    pub fn filename(&self) -> String {
        format!("{}.bin", self.id.as_str())
    }
}

/// Frozen catalog. Keeping it in source has two benefits: the SHA-256
/// values cannot be tampered with at runtime, and an offline machine
/// can still iterate the picker.
///
/// The `sha256` strings here are intentionally **left empty for now**
/// (release blocker): they must be filled in with the verified upstream
/// digests before the voice-input feature is enabled in a release. The
/// downloader treats an empty digest as "skip verification + log a
/// warning" so development and CI flows work, but the warning makes it
/// impossible to ship an empty hash silently. See [`download`] for the
/// runtime handling.
pub fn entries() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            id: ModelId::from("ggml-tiny-q5_1"),
            display: "Whisper Tiny (저사양, 다국어)".into(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin"
                .into(),
            sha256: String::new(),
            size_bytes: 31_000_000,
            languages: ModelLanguages::Multilingual,
            recommendation: "저사양".into(),
        },
        ModelEntry {
            id: ModelId::from("ggml-base-q5_1"),
            display: "Whisper Base (균형, 다국어)".into(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin"
                .into(),
            sha256: String::new(),
            size_bytes: 58_000_000,
            languages: ModelLanguages::Multilingual,
            recommendation: "균형".into(),
        },
        ModelEntry {
            id: ModelId::from("ggml-small-q5_1"),
            display: "Whisper Small (권장, 다국어)".into(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin"
                .into(),
            sha256: String::new(),
            size_bytes: 190_000_000,
            languages: ModelLanguages::Multilingual,
            recommendation: "권장".into(),
        },
        ModelEntry {
            id: ModelId::from("ggml-medium-q5_0"),
            display: "Whisper Medium (고품질, 다국어)".into(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin"
                .into(),
            sha256: String::new(),
            size_bytes: 539_000_000,
            languages: ModelLanguages::Multilingual,
            recommendation: "고품질".into(),
        },
    ]
}

/// Locate a single entry by id. Returns `None` if the catalog has been
/// rebuilt without a previously-saved id; the UI should fall back to
/// the recommended default.
pub fn find(id: &ModelId) -> Option<ModelEntry> {
    entries().into_iter().find(|e| &e.id == id)
}

/// The recommended default when no choice has been persisted yet. Small
/// q5 strikes the best quality/size balance for Korean speech on a
/// laptop-class CPU.
pub fn recommended_default() -> ModelEntry {
    find(&ModelId::from("ggml-small-q5_1"))
        .or_else(|| entries().into_iter().next())
        .expect("catalog must have at least one entry")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_unique() {
        let rows = entries();
        assert!(!rows.is_empty());
        let mut ids: Vec<_> = rows.iter().map(|r| r.id.as_str().to_string()).collect();
        ids.sort();
        let original_len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "duplicate model ids in catalog");
    }

    #[test]
    fn every_url_is_https_huggingface() {
        for entry in entries() {
            assert!(
                entry.url.starts_with("https://huggingface.co/"),
                "non-trusted host in catalog: {}",
                entry.url
            );
        }
    }

    #[test]
    fn sha256_strings_are_either_empty_or_64_lowercase_hex() {
        for entry in entries() {
            if entry.sha256.is_empty() {
                continue;
            }
            assert_eq!(
                entry.sha256.len(),
                64,
                "bad sha256 length on {:?}",
                entry.id
            );
            assert!(
                entry
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "sha256 must be lowercase hex: {}",
                entry.sha256
            );
        }
    }

    #[test]
    fn recommended_default_is_in_catalog() {
        let recommended = recommended_default();
        assert!(entries().iter().any(|e| e.id == recommended.id));
    }

    #[test]
    fn modelid_roundtrips_through_serde() {
        let id = ModelId::from("ggml-small-q5_1");
        let s = serde_json::to_string(&id).unwrap();
        assert_eq!(s, "\"ggml-small-q5_1\"");
        let back: ModelId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn filename_uses_id_with_bin_extension() {
        let entry = recommended_default();
        assert_eq!(entry.filename(), format!("{}.bin", entry.id.as_str()));
    }
}
