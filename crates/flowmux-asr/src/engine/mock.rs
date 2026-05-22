// SPDX-License-Identifier: GPL-3.0-or-later
//! Mock engine used when the `whisper-engine` feature is off.
//!
//! The mock returns a deterministic string that describes the input it
//! was given so integration tests and the headless workspace check can
//! exercise the push-to-talk plumbing without compiling whisper.cpp.

use super::{AsrEngine, AsrEngineError, Transcription};
use crate::config::AsrEngineConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct MockEngine {
    config: AsrEngineConfig,
}

impl MockEngine {
    pub fn new(config: AsrEngineConfig) -> Self {
        Self { config }
    }
}

impl AsrEngine for MockEngine {
    fn transcribe(
        &self,
        pcm: &[f32],
        cancel: Arc<AtomicBool>,
    ) -> Result<Transcription, AsrEngineError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(AsrEngineError::Cancelled);
        }
        let seconds = pcm.len() as f32 / crate::audio::TARGET_SAMPLE_RATE as f32;
        Ok(Transcription {
            text: format!(
                "[mock asr: {:.2}s, lang={}]",
                seconds,
                self.config.language.whisper_code()
            ),
            language: self.config.language.whisper_code().to_string(),
            seconds_processed: seconds,
        })
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelLanguages;
    use crate::config::Language;
    use std::path::PathBuf;

    fn cfg() -> AsrEngineConfig {
        AsrEngineConfig::new(PathBuf::from("unused"), ModelLanguages::Multilingual)
            .with_language(Language::Code("ko".into()))
    }

    #[test]
    fn mock_returns_duration_label() {
        let engine = MockEngine::new(cfg());
        let pcm = vec![0.0_f32; 16_000]; // 1 s
        let cancel = Arc::new(AtomicBool::new(false));
        let out = engine.transcribe(&pcm, cancel).unwrap();
        assert!(out.text.contains("1.00s"));
        assert!(out.text.contains("lang=ko"));
        assert_eq!(out.language, "ko");
    }

    #[test]
    fn mock_respects_cancel_flag() {
        let engine = MockEngine::new(cfg());
        let cancel = Arc::new(AtomicBool::new(true));
        let err = engine
            .transcribe(&[0.0_f32; 1024], cancel)
            .expect_err("cancel must abort");
        assert!(matches!(err, AsrEngineError::Cancelled));
    }
}
