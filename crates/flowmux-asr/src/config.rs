// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime configuration consumed by the engine layer.
//!
//! Separate from the user-facing options in `flowmux-config::options`
//! so the engine can be exercised in tests without dragging the entire
//! XDG config plumbing in.

use crate::catalog::ModelLanguages;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One-language identifier in the BCP-47 short form Whisper accepts
/// (`"ko"`, `"en"`, `"ja"`, …) or [`Language::Auto`] for automatic
/// detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Language {
    Auto,
    Code(String),
}

impl Default for Language {
    fn default() -> Self {
        Self::Auto
    }
}

impl Language {
    /// Whisper accepts `"auto"` or a two-letter code; this maps both
    /// branches to the wire form.
    pub fn whisper_code(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::Code(s) => s.as_str(),
        }
    }
}

/// Inference configuration for [`crate::engine::AsrEngine`].
#[derive(Debug, Clone)]
pub struct AsrEngineConfig {
    pub model_path: PathBuf,
    pub languages: ModelLanguages,
    pub language: Language,
    /// `0` means "let the engine pick" — for whisper.cpp that becomes
    /// `min(8, num_cpus - 2)` so the GUI thread stays responsive on
    /// laptop-class CPUs.
    pub threads: i32,
    /// When true the engine ignores `language` and translates to
    /// English. Whisper supports this natively.
    pub translate_to_english: bool,
}

impl AsrEngineConfig {
    pub fn new(model_path: PathBuf, languages: ModelLanguages) -> Self {
        Self {
            model_path,
            languages,
            language: Language::Auto,
            threads: 0,
            translate_to_english: false,
        }
    }

    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    pub fn with_threads(mut self, threads: i32) -> Self {
        self.threads = threads.max(0);
        self
    }

    pub fn with_translate(mut self, translate: bool) -> Self {
        self.translate_to_english = translate;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_serializes_with_kind_tag() {
        let auto = Language::Auto;
        let s = serde_json::to_string(&auto).unwrap();
        assert!(s.contains("\"kind\":\"auto\""));

        let ko = Language::Code("ko".into());
        let s = serde_json::to_string(&ko).unwrap();
        assert!(s.contains("\"kind\":\"code\""));
        assert!(s.contains("\"value\":\"ko\""));
    }

    #[test]
    fn whisper_code_maps_auto_and_codes() {
        assert_eq!(Language::Auto.whisper_code(), "auto");
        assert_eq!(Language::Code("ko".into()).whisper_code(), "ko");
    }

    #[test]
    fn engine_config_clamps_threads_at_zero() {
        let cfg = AsrEngineConfig::new(PathBuf::from("/tmp/m.bin"), ModelLanguages::Multilingual)
            .with_threads(-4);
        assert_eq!(cfg.threads, 0);
    }
}
