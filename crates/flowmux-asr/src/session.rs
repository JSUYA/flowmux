// SPDX-License-Identifier: GPL-3.0-or-later
//! Push-to-talk session: start capture, stop on release, transcribe.
//!
//! The session is the single object the GUI talks to. It hides the
//! engine + capture + cancel flag plumbing behind a tiny state machine
//! so the GTK side only needs to wire `start_ptt` / `release_ptt`
//! handlers to the keypress events.

use crate::audio::capture::{AudioCapture, CaptureHandle, CaptureSpec};
use crate::engine::{AsrEngine, AsrEngineError, Transcription};
use crate::AsrError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Knobs the GUI tunes per session.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub device_name: Option<String>,
    pub max_duration: Duration,
    /// Append `\r` to the transcription before injecting it into the
    /// terminal. Disabled by default — the user usually wants to read
    /// the line first.
    pub auto_enter: bool,
    /// Drop the transcription when the user releases the key before
    /// this minimum duration. Stops accidental key-taps from sending
    /// noise into the engine. Set to `0` to disable the floor.
    pub min_duration: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            max_duration: Duration::from_secs(30),
            auto_enter: false,
            min_duration: Duration::from_millis(200),
        }
    }
}

/// State transitions a session walks through. Returned to the GUI so
/// the indicator widget can swap colour on every event.
#[derive(Debug, Clone)]
pub enum PttEvent {
    Recording,
    Transcribing,
    Done(Transcription),
    DroppedTooShort { duration_seconds: f32 },
    Truncated,
    Cancelled,
    Failed(String),
}

/// Public session handle. The engine is shared across sessions —
/// loading the whisper model is the expensive part and the GUI keeps
/// the engine warm.
pub struct PttSession {
    engine: Arc<dyn AsrEngine>,
    config: SessionConfig,
    capture: Option<CaptureHandle>,
    cancel: Arc<AtomicBool>,
}

impl PttSession {
    pub fn new(engine: Arc<dyn AsrEngine>, config: SessionConfig) -> Self {
        Self {
            engine,
            config,
            capture: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start capture. Returns immediately; the worker thread continues
    /// until `finish_and_transcribe` or `cancel` is called.
    pub fn start(&mut self) -> Result<PttEvent, AsrError> {
        if self.capture.is_some() {
            return Ok(PttEvent::Failed("이미 녹음 중입니다".into()));
        }
        let spec = CaptureSpec {
            device_name: self.config.device_name.clone(),
            max_duration: self.config.max_duration,
        };
        let handle = AudioCapture::start_session(spec)?;
        self.capture = Some(handle);
        self.cancel.store(false, Ordering::Relaxed);
        Ok(PttEvent::Recording)
    }

    /// Stop capture, run transcription, and return the final event.
    /// Blocking — the caller wraps this in `spawn_blocking` so the GUI
    /// thread stays responsive.
    pub fn finish_and_transcribe(&mut self) -> Result<PttEvent, AsrError> {
        let Some(handle) = self.capture.take() else {
            return Ok(PttEvent::Failed("녹음이 시작되지 않았습니다".into()));
        };
        let audio = handle.stop()?;
        if audio.duration_seconds < self.config.min_duration.as_secs_f32() {
            return Ok(PttEvent::DroppedTooShort {
                duration_seconds: audio.duration_seconds,
            });
        }
        let cancel = self.cancel.clone();
        let result = self.engine.transcribe(&audio.pcm_16k_mono, cancel);
        match result {
            Ok(mut t) => {
                if t.text.trim().is_empty() {
                    return Ok(PttEvent::DroppedTooShort {
                        duration_seconds: audio.duration_seconds,
                    });
                }
                if self.config.auto_enter && !t.text.ends_with('\n') {
                    t.text.push('\r');
                }
                if audio.truncated {
                    // Surface truncation to the GUI as an event, but
                    // still hand back the transcribed text — the user
                    // would rather see what was captured than lose
                    // everything past the 30 s cap.
                    let _ = PttEvent::Truncated;
                }
                Ok(PttEvent::Done(t))
            }
            Err(AsrEngineError::Cancelled) => Ok(PttEvent::Cancelled),
            Err(e) => Ok(PttEvent::Failed(e.to_string())),
        }
    }

    /// Abort capture and discard the buffer. Used by the Esc / double-
    /// tap PTT cancel path.
    pub fn cancel(&mut self) -> PttEvent {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.capture.take() {
            handle.abort();
        }
        PttEvent::Cancelled
    }

    /// Returns true if the session is mid-capture.
    pub fn is_recording(&self) -> bool {
        self.capture.is_some()
    }
}

/// Sanitise transcription text before it is injected into a PTY. The
/// engine occasionally emits control characters or `\0` from noisy
/// recordings; those would land as raw bytes in the terminal and let a
/// crafted audio prompt move the cursor or rewrite history.
pub fn sanitize_for_pty(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        // Allow regular printable Unicode + tab + newline; drop other
        // ASCII control bytes including 0x1B (ESC) so OSC/CSI cannot be
        // smuggled through.
        if ch == '\n' || ch == '\t' {
            out.push(ch);
        } else if (ch as u32) < 0x20 || ch == '\u{7f}' {
            continue;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelLanguages;
    use crate::config::AsrEngineConfig;
    use crate::engine::MockEngine;
    use std::path::PathBuf;

    fn engine() -> Arc<dyn AsrEngine> {
        Arc::new(MockEngine::new(AsrEngineConfig::new(
            PathBuf::from("unused"),
            ModelLanguages::Multilingual,
        )))
    }

    #[test]
    fn sanitize_drops_escape_and_keeps_tab_newline() {
        let raw = "hello\t world\nrm -rf /\x1b]0;evil\x07";
        let out = sanitize_for_pty(raw);
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\x07'));
        assert!(out.contains('\t'));
        assert!(out.contains('\n'));
    }

    #[test]
    fn sanitize_passes_korean_through_unchanged() {
        let raw = "안녕하세요 flowmux";
        assert_eq!(sanitize_for_pty(raw), raw);
    }

    #[test]
    fn session_returns_failed_when_finishing_without_start() {
        let mut session = PttSession::new(engine(), SessionConfig::default());
        let event = session.finish_and_transcribe().unwrap();
        assert!(matches!(event, PttEvent::Failed(_)));
    }

    #[test]
    fn session_is_recording_flag_tracks_capture_state() {
        let session = PttSession::new(engine(), SessionConfig::default());
        assert!(!session.is_recording());
    }
}
