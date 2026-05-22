// SPDX-License-Identifier: GPL-3.0-or-later
//! Push-to-talk controller: glues the [`flowmux_asr`] engine to the
//! GTK main thread.
//!
//! The controller is created once at startup and lives behind an
//! `Rc<RefCell<…>>` shared by the keybinding action handler, the
//! headerbar mic button, and the options dialog. It is GTK-thread-
//! affine so widget updates can happen synchronously; long-running
//! work (model load, transcription) is offloaded to tokio's
//! `spawn_blocking` pool and the result is delivered back through an
//! `async_channel` that the dispatch loop drains on the main thread.

use crate::keybindings::{FocusedPane, TerminalRegistry};
use crate::ui::window::ClipboardToast;
use adw::prelude::*;
use flowmux_asr::audio::TARGET_SAMPLE_RATE;
use flowmux_asr::catalog::{self, ModelEntry};
use flowmux_asr::config::{AsrEngineConfig, Language};
use flowmux_asr::engine::{load_engine, AsrEngine};
use flowmux_asr::session::{sanitize_for_pty, PttEvent, PttSession, SessionConfig};
use flowmux_asr::ModelStore;
use flowmux_config::asr::{AsrLanguage, AsrOptions, PttMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// State emitted to the headerbar indicator + toast layer.
#[derive(Debug, Clone)]
pub enum AsrUiEvent {
    Recording,
    Transcribing,
    Done { text: String, language: String },
    DroppedTooShort { seconds: f32 },
    Failed(String),
    Cancelled,
}

/// Pure-Rust controller shell. GTK widgets that subscribe to status
/// changes use [`AsrController::set_event_handler`] to register one
/// callback fired on every state transition.
pub struct AsrController {
    options: AsrOptions,
    runtime: tokio::runtime::Handle,
    store: ModelStore,
    engine: Option<Arc<dyn AsrEngine>>,
    engine_signature: Option<String>,
    session: Option<PttSession>,
    event_tx: async_channel::Sender<AsrUiEvent>,
    event_rx: Option<async_channel::Receiver<AsrUiEvent>>,
    focused: FocusedPane,
    registry: TerminalRegistry,
    clipboard_toast: ClipboardToast,
}

/// Convenience type — the controller is always shared by reference,
/// never sent across threads.
pub type AsrControllerHandle = Rc<RefCell<AsrController>>;

impl AsrController {
    pub fn new(
        options: AsrOptions,
        runtime: tokio::runtime::Handle,
        focused: FocusedPane,
        registry: TerminalRegistry,
        clipboard_toast: ClipboardToast,
    ) -> AsrControllerHandle {
        let (event_tx, event_rx) = async_channel::unbounded::<AsrUiEvent>();
        let store = ModelStore::xdg_default().unwrap_or_else(|| {
            // Fall back to a temp store rather than panic — the
            // options dialog reports the error inline when the user
            // tries to download a model.
            ModelStore::new(std::env::temp_dir().join("flowmux-asr-models"))
        });
        let _ = store.ensure_dir();
        Rc::new(RefCell::new(Self {
            options,
            runtime,
            store,
            engine: None,
            engine_signature: None,
            session: None,
            event_tx,
            event_rx: Some(event_rx),
            focused,
            registry,
            clipboard_toast,
        }))
    }

    /// Hand the receiver off to the dispatch loop exactly once. The
    /// caller spawns a `glib::MainContext::spawn_local` that drains it
    /// and updates UI in response to each event.
    pub fn take_event_receiver(&mut self) -> Option<async_channel::Receiver<AsrUiEvent>> {
        self.event_rx.take()
    }

    pub fn options(&self) -> &AsrOptions {
        &self.options
    }

    /// Push an updated [`AsrOptions`] snapshot. If the model changed
    /// the cached engine is dropped so the next session loads the
    /// new one.
    pub fn replace_options(&mut self, options: AsrOptions) {
        if options.active_model_id != self.options.active_model_id
            || options.language.as_code() != self.options.language.as_code()
            || options.translate_to_english != self.options.translate_to_english
        {
            self.engine = None;
            self.engine_signature = None;
        }
        self.options = options;
    }

    pub fn is_recording(&self) -> bool {
        self.session
            .as_ref()
            .map(|s| s.is_recording())
            .unwrap_or(false)
    }

    /// Entry point for the keybinding action / mic button. Treats the
    /// call as a toggle in [`PttMode::Toggle`] mode and as a start in
    /// [`PttMode::Hold`] mode. Hold mode pairs this with
    /// [`Self::release`] on key-release.
    pub fn activate(&mut self) {
        if !self.options.is_ready() {
            self.emit(AsrUiEvent::Failed(
                "음성 입력이 비활성화되어 있거나 모델이 선택되지 않았습니다.".into(),
            ));
            return;
        }
        if self.is_recording() {
            // Either second tap in Toggle, or accidental autorepeat.
            // Finish the session in both cases.
            self.finish();
            return;
        }
        if self.start().is_err() {
            // Already surfaced through emit() inside start().
        }
    }

    /// Pair of [`Self::activate`] for [`PttMode::Hold`] — fired when
    /// the accelerator's key is released.
    pub fn release(&mut self) {
        if self.options.ptt_mode != PttMode::Hold {
            return;
        }
        if self.is_recording() {
            self.finish();
        }
    }

    pub fn cancel(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = session.cancel();
        }
        self.emit(AsrUiEvent::Cancelled);
    }

    fn start(&mut self) -> Result<(), ()> {
        let Some(entry) = self.active_entry() else {
            self.emit(AsrUiEvent::Failed("선택된 모델을 찾을 수 없습니다.".into()));
            return Err(());
        };
        if !self.store.is_installed(&entry) {
            self.emit(AsrUiEvent::Failed(
                "모델이 디스크에 설치되지 않았습니다. 설정에서 다운로드를 진행하세요.".into(),
            ));
            return Err(());
        }
        let engine = match self.ensure_engine(&entry) {
            Ok(e) => e,
            Err(msg) => {
                self.emit(AsrUiEvent::Failed(msg));
                return Err(());
            }
        };
        let session_config = SessionConfig {
            device_name: self.options.input_device.clone(),
            max_duration: Duration::from_secs(self.options.max_seconds as u64),
            auto_enter: self.options.auto_enter,
            min_duration: Duration::from_millis(250),
        };
        let mut session = PttSession::new(engine, session_config);
        match session.start() {
            Ok(_) => {
                self.session = Some(session);
                self.emit(AsrUiEvent::Recording);
                Ok(())
            }
            Err(e) => {
                self.emit(AsrUiEvent::Failed(format!("녹음 시작 실패: {e}")));
                Err(())
            }
        }
    }

    fn finish(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        self.emit(AsrUiEvent::Transcribing);
        let tx = self.event_tx.clone();
        // Move the (Send) session onto a tokio blocking worker so the
        // whisper.cpp call cannot stall the GTK main loop.
        self.runtime.spawn_blocking(move || {
            let result = session.finish_and_transcribe();
            let event = match result {
                Ok(PttEvent::Done(t)) => AsrUiEvent::Done {
                    text: sanitize_for_pty(&t.text),
                    language: t.language,
                },
                Ok(PttEvent::DroppedTooShort { duration_seconds }) => AsrUiEvent::DroppedTooShort {
                    seconds: duration_seconds,
                },
                Ok(PttEvent::Cancelled) => AsrUiEvent::Cancelled,
                Ok(PttEvent::Failed(msg)) => AsrUiEvent::Failed(msg),
                Ok(other) => AsrUiEvent::Failed(format!("unexpected state: {other:?}")),
                Err(e) => AsrUiEvent::Failed(format!("transcription failed: {e}")),
            };
            let _ = tx.send_blocking(event);
        });
    }

    fn ensure_engine(&mut self, entry: &ModelEntry) -> Result<Arc<dyn AsrEngine>, String> {
        let signature = format!(
            "{}::{}::{}",
            entry.id.as_str(),
            self.options.language.as_code(),
            self.options.translate_to_english
        );
        if let Some(cached) = &self.engine {
            if self.engine_signature.as_deref() == Some(signature.as_str()) {
                return Ok(cached.clone());
            }
        }
        let path = self.store.model_path(entry);
        let language = match &self.options.language {
            AsrLanguage::Auto => Language::Auto,
            AsrLanguage::Code(c) => Language::Code(c.clone()),
        };
        let cfg = AsrEngineConfig::new(path, entry.languages)
            .with_language(language)
            .with_translate(self.options.translate_to_english);
        let engine = load_engine(cfg).map_err(|e| format!("엔진 로드 실패: {e}"))?;
        let engine: Arc<dyn AsrEngine> = Arc::from(engine);
        self.engine = Some(engine.clone());
        self.engine_signature = Some(signature);
        Ok(engine)
    }

    fn active_entry(&self) -> Option<ModelEntry> {
        let id = self.options.active_model_id.clone();
        catalog::entries().into_iter().find(|e| e.id.as_str() == id)
    }

    fn emit(&self, event: AsrUiEvent) {
        let _ = self.event_tx.send_blocking(event);
    }
}

/// Take an [`AsrUiEvent`] received on the GTK main thread and apply
/// its side effects: inject text into the focused terminal, update the
/// toast, push the indicator state. Pulled out of the controller so
/// it can run inside `spawn_local` without holding the borrow.
pub fn handle_ui_event(
    event: AsrUiEvent,
    focused: &FocusedPane,
    registry: &TerminalRegistry,
    clipboard_toast: &ClipboardToast,
    indicator: &dyn AsrIndicator,
) {
    match event {
        AsrUiEvent::Recording => {
            indicator.set_recording(true);
            clipboard_toast.show_with_message("🎤 녹음 중… 키를 떼면 인식됩니다");
        }
        AsrUiEvent::Transcribing => {
            indicator.set_busy(true);
        }
        AsrUiEvent::Done { text, language: _ } => {
            indicator.set_recording(false);
            indicator.set_busy(false);
            inject_text(focused, registry, clipboard_toast, &text);
        }
        AsrUiEvent::DroppedTooShort { seconds } => {
            indicator.set_recording(false);
            indicator.set_busy(false);
            clipboard_toast.show_with_message(&format!(
                "음성이 너무 짧습니다 ({:.2}초). 다시 시도하세요.",
                seconds
            ));
        }
        AsrUiEvent::Failed(msg) => {
            indicator.set_recording(false);
            indicator.set_busy(false);
            clipboard_toast.show_with_message(&format!("음성 입력 실패: {msg}"));
        }
        AsrUiEvent::Cancelled => {
            indicator.set_recording(false);
            indicator.set_busy(false);
            clipboard_toast.show_with_message("음성 입력이 취소되었습니다");
        }
    }
}

fn inject_text(
    focused: &FocusedPane,
    registry: &TerminalRegistry,
    clipboard_toast: &ClipboardToast,
    text: &str,
) {
    let trimmed = text.trim_end_matches('\n');
    if trimmed.is_empty() {
        clipboard_toast.show_with_message("인식된 음성이 없습니다");
        return;
    }
    let Some(pane_id) = focused.get() else {
        clipboard_toast.show_with_message("포커스된 pane이 없습니다");
        return;
    };
    let registry = registry.borrow();
    let Some(terminal) = registry.active_terminal(pane_id) else {
        clipboard_toast.show_with_message("터미널 pane이 포커스되어 있어야 음성 입력이 동작합니다");
        return;
    };
    terminal.feed_text(text);
    clipboard_toast.show_with_message(&format!("🎤 입력됨: {}", preview(trimmed)));
}

fn preview(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX).collect();
    out.push('…');
    out
}

/// Tiny trait so the controller can talk to whatever indicator widget
/// the headerbar exposes without coupling to a concrete `Button` type.
pub trait AsrIndicator {
    fn set_recording(&self, on: bool);
    fn set_busy(&self, on: bool);
}

/// Default indicator backed by a single [`gtk::Button`]. The button
/// gains the `flowmux-asr-recording` CSS class while a session is
/// active so a stylesheet can paint it red.
pub struct ButtonIndicator {
    pub button: gtk::Button,
}

impl AsrIndicator for ButtonIndicator {
    fn set_recording(&self, on: bool) {
        if on {
            self.button.add_css_class("flowmux-asr-recording");
        } else {
            self.button.remove_css_class("flowmux-asr-recording");
        }
    }
    fn set_busy(&self, on: bool) {
        if on {
            self.button.add_css_class("flowmux-asr-busy");
        } else {
            self.button.remove_css_class("flowmux-asr-busy");
        }
    }
}

/// Whisper accepts up to 30 s in one shot; the controller surfaces
/// that as a status hint for the headerbar.
pub fn max_buffer_seconds(opts: &AsrOptions) -> u16 {
    AsrOptions::clamp_max_seconds(opts.max_seconds)
        .min((30_u32.saturating_mul(TARGET_SAMPLE_RATE) / TARGET_SAMPLE_RATE) as u16)
}
