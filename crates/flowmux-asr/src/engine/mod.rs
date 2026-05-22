// SPDX-License-Identifier: GPL-3.0-or-later
//! ASR engine trait and concrete implementations.
//!
//! The trait is intentionally narrow: every engine consumes 16 kHz mono
//! `f32` PCM and returns one [`Transcription`]. Push-to-talk is the
//! only mode supported in this milestone, so streaming/partials are
//! out of scope for the trait.

use crate::config::AsrEngineConfig;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

mod mock;
#[cfg(feature = "whisper-engine")]
mod whisper;

pub use mock::MockEngine;
#[cfg(feature = "whisper-engine")]
pub use whisper::WhisperEngine;

#[derive(Debug, thiserror::Error)]
pub enum AsrEngineError {
    #[error("model file not found: {0}")]
    ModelMissing(String),
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("cancelled")]
    Cancelled,
}

/// Result of a single push-to-talk transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    /// Whitespace-trimmed text. Empty when the engine emitted only
    /// `[BLANK_AUDIO]` or silence markers.
    pub text: String,
    /// Detected (or configured) language code. `"auto"` is replaced
    /// with the engine's best guess when the engine supports it.
    pub language: String,
    pub seconds_processed: f32,
}

impl Transcription {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            language: "auto".into(),
            seconds_processed: 0.0,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// One engine instance owns a loaded model. Implementations are
/// `Send + Sync` so the engine can be parked on a tokio worker.
pub trait AsrEngine: Send + Sync {
    /// Transcribe one PCM buffer. `cancel` is polled periodically; the
    /// implementation should bail with [`AsrEngineError::Cancelled`]
    /// when it flips to true.
    fn transcribe(
        &self,
        pcm_16k_mono: &[f32],
        cancel: Arc<AtomicBool>,
    ) -> Result<Transcription, AsrEngineError>;

    /// Display name used by debug logs and error toasts.
    fn name(&self) -> &str;
}

/// Build the engine the user requested. When `whisper-engine` is off
/// the catalog can still be exercised end-to-end through the mock,
/// which is what the headless `cargo check --workspace` path uses.
pub fn load_engine(config: AsrEngineConfig) -> Result<Box<dyn AsrEngine>, AsrEngineError> {
    if !config.model_path.exists() {
        return Err(AsrEngineError::ModelMissing(
            config.model_path.display().to_string(),
        ));
    }
    #[cfg(feature = "whisper-engine")]
    {
        let engine = WhisperEngine::load(config)?;
        Ok(Box::new(engine))
    }
    #[cfg(not(feature = "whisper-engine"))]
    {
        Ok(Box::new(MockEngine::new(config)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_is_blank_when_text_is_whitespace() {
        let t = Transcription {
            text: "   \n  ".into(),
            language: "ko".into(),
            seconds_processed: 1.0,
        };
        assert!(t.is_blank());

        let t = Transcription {
            text: "hello".into(),
            language: "en".into(),
            seconds_processed: 1.0,
        };
        assert!(!t.is_blank());
    }
}
