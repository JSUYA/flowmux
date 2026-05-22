// SPDX-License-Identifier: GPL-3.0-or-later
//! Whisper.cpp backed engine.
//!
//! Compiled only when the `whisper-engine` feature is on. The GTK GUI
//! crate turns the feature on so production builds get real speech
//! recognition; the headless workspace check stays light by skipping
//! the C++ build.

use super::{AsrEngine, AsrEngineError, Transcription};
use crate::config::AsrEngineConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperEngine {
    ctx: Mutex<WhisperContext>,
    config: AsrEngineConfig,
}

impl WhisperEngine {
    pub fn load(config: AsrEngineConfig) -> Result<Self, AsrEngineError> {
        let path_str = config.model_path.to_string_lossy().to_string();
        let ctx = WhisperContext::new_with_params(&path_str, WhisperContextParameters::default())
            .map_err(|e| AsrEngineError::ModelLoad(e.to_string()))?;
        Ok(Self {
            ctx: Mutex::new(ctx),
            config,
        })
    }

    fn effective_threads(&self) -> i32 {
        if self.config.threads > 0 {
            return self.config.threads;
        }
        let n = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);
        (n - 2).clamp(2, 8)
    }
}

impl AsrEngine for WhisperEngine {
    fn transcribe(
        &self,
        pcm: &[f32],
        cancel: Arc<AtomicBool>,
    ) -> Result<Transcription, AsrEngineError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(AsrEngineError::Cancelled);
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.effective_threads());
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_translate(self.config.translate_to_english);
        let lang_code = self.config.language.whisper_code().to_string();
        if lang_code != "auto" {
            params.set_language(Some(Box::leak(lang_code.clone().into_boxed_str())));
        }

        let mut ctx = self
            .ctx
            .lock()
            .map_err(|_| AsrEngineError::Inference("mutex poisoned".into()))?;
        let mut state = ctx
            .create_state()
            .map_err(|e| AsrEngineError::Inference(format!("create_state: {e}")))?;

        state
            .full(params, pcm)
            .map_err(|e| AsrEngineError::Inference(format!("full: {e}")))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(AsrEngineError::Cancelled);
        }

        let num_segments = state
            .full_n_segments()
            .map_err(|e| AsrEngineError::Inference(format!("n_segments: {e}")))?;
        let mut text = String::new();
        for i in 0..num_segments {
            let seg = state
                .full_get_segment_text(i)
                .map_err(|e| AsrEngineError::Inference(format!("segment_text: {e}")))?;
            text.push_str(&seg);
        }
        let text = text.trim().to_string();
        // whisper-rs 0.13 no longer exposes a stable language-detection
        // accessor on `WhisperState`; we report whatever was requested
        // by config. Auto-detect surfaces back through Whisper's
        // internal handling regardless.
        let detected = lang_code.clone();
        let seconds = pcm.len() as f32 / crate::audio::TARGET_SAMPLE_RATE as f32;
        Ok(Transcription {
            text,
            language: detected,
            seconds_processed: seconds,
        })
    }

    fn name(&self) -> &str {
        "whisper.cpp"
    }
}
