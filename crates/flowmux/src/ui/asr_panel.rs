// SPDX-License-Identifier: GPL-3.0-or-later
//! "Voice input" tab inside the options dialog.
//!
//! The widget tree built here mutates an [`AsrOptions`] in the
//! `RefCell` handed in by the dialog; the dialog reads it back when the
//! user clicks OK and persists it to `options.json`.
//!
//! Layout, top-to-bottom:
//!
//! * Enable switch + status line ("모델 미설치" / "준비됨").
//! * Model dropdown sourced from [`flowmux_asr::catalog`].
//! * Language dropdown (`auto` + a curated list of Whisper languages).
//! * "Microphone permission" group with a "Test microphone" button
//!   that runs [`flowmux_asr::audio::probe_microphone`] and surfaces
//!   the result inline.
//! * Auto-Enter switch.
//! * PTT mode switch (Hold vs Toggle).

use adw::prelude::*;
use flowmux_asr::audio::{probe_microphone, MicProbeOutcome};
use flowmux_asr::catalog::{self, ModelEntry};
use flowmux_asr::ModelStore;
use flowmux_config::asr::{AsrLanguage, AsrOptions, PttMode};
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

const SHORTCUT_HINT: &str = "단축키: Ctrl+Alt+Space (Keybindings 탭에서 변경)";

/// One row in the language dropdown. `code = "auto"` means the engine
/// picks automatically.
struct LanguageRow {
    code: &'static str,
    label: &'static str,
}

const LANGUAGES: &[LanguageRow] = &[
    LanguageRow {
        code: "auto",
        label: "자동 감지 (Whisper)",
    },
    LanguageRow {
        code: "ko",
        label: "한국어",
    },
    LanguageRow {
        code: "en",
        label: "English",
    },
    LanguageRow {
        code: "ja",
        label: "日本語",
    },
    LanguageRow {
        code: "zh",
        label: "中文",
    },
];

/// Build the Voice tab body. Widgets write back into `state` whenever
/// the user toggles them, so the options dialog's OK handler picks up
/// the latest snapshot from the `RefCell`.
pub fn build(state: Rc<RefCell<AsrOptions>>) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(20);
    outer.set_margin_end(20);

    let intro = gtk::Label::new(Some(
        "마이크를 누르고 말하면 인식된 텍스트가 포커스된 터미널 pane에 입력됩니다. \
         모든 음성 처리는 디바이스 안에서 일어나며 외부 서버로 전송되지 않습니다.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    outer.append(&intro);

    let enable_switch = build_enable_switch(state.clone());
    outer.append(&labelled("음성 입력 사용", &enable_switch));

    let shortcut_hint = gtk::Label::new(Some(SHORTCUT_HINT));
    shortcut_hint.set_wrap(true);
    shortcut_hint.set_xalign(0.0);
    shortcut_hint.add_css_class("dim-label");
    outer.append(&shortcut_hint);

    let (model_dropdown, model_status) = build_model_row(state.clone());
    outer.append(&labelled("모델", &model_dropdown));
    outer.append(&model_status);

    let language_dropdown = build_language_dropdown(state.clone());
    outer.append(&labelled("언어", &language_dropdown));

    outer.append(&build_microphone_group(state.clone()));

    let auto_enter_switch = build_auto_enter_switch(state.clone());
    outer.append(&labelled("결과 끝에 Enter 자동 입력", &auto_enter_switch));

    let ptt_mode_dropdown = build_ptt_mode_dropdown(state.clone());
    outer.append(&labelled("푸시-투-토크 동작", &ptt_mode_dropdown));

    outer
}

fn labelled(text: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}

fn build_enable_switch(state: Rc<RefCell<AsrOptions>>) -> gtk::Switch {
    let sw = gtk::Switch::new();
    sw.set_active(state.borrow().enabled);
    sw.connect_active_notify(move |s| {
        state.borrow_mut().enabled = s.is_active();
    });
    sw
}

fn build_auto_enter_switch(state: Rc<RefCell<AsrOptions>>) -> gtk::Switch {
    let sw = gtk::Switch::new();
    sw.set_active(state.borrow().auto_enter);
    sw.connect_active_notify(move |s| {
        state.borrow_mut().auto_enter = s.is_active();
    });
    sw
}

fn build_model_row(state: Rc<RefCell<AsrOptions>>) -> (gtk::DropDown, gtk::Label) {
    let entries = catalog::entries();
    let labels: Vec<String> = entries.iter().map(|e| e.display.clone()).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let dropdown = gtk::DropDown::from_strings(&label_refs);
    let current_id = state.borrow().active_model_id.clone();
    let idx = entries
        .iter()
        .position(|e| e.id.as_str() == current_id)
        .unwrap_or(0);
    dropdown.set_selected(idx as u32);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    refresh_model_status(&status, entries.get(idx));

    {
        let state = state.clone();
        let status = status.clone();
        let entries = entries.clone();
        dropdown.connect_selected_notify(move |d| {
            let i = d.selected() as usize;
            if let Some(entry) = entries.get(i) {
                state.borrow_mut().active_model_id = entry.id.as_str().to_string();
                refresh_model_status(&status, Some(entry));
            }
        });
    }

    (dropdown, status)
}

fn refresh_model_status(status: &gtk::Label, entry: Option<&ModelEntry>) {
    let Some(entry) = entry else {
        status.set_text("");
        return;
    };
    let mb = (entry.size_bytes as f32 / 1_000_000.0).round();
    let installed = match ModelStore::xdg_default() {
        Some(store) => store.is_installed(entry),
        None => false,
    };
    let state = if installed {
        "설치됨"
    } else {
        "다운로드 필요"
    };
    status.set_text(&format!("크기: {mb} MB · 상태: {state}"));
}

fn build_language_dropdown(state: Rc<RefCell<AsrOptions>>) -> gtk::DropDown {
    let labels: Vec<&str> = LANGUAGES.iter().map(|r| r.label).collect();
    let dropdown = gtk::DropDown::from_strings(&labels);
    let current_code = state.borrow().language.as_code().to_string();
    let idx = LANGUAGES
        .iter()
        .position(|r| r.code == current_code)
        .unwrap_or(0);
    dropdown.set_selected(idx as u32);
    let state_clone = state.clone();
    dropdown.connect_selected_notify(move |d| {
        let i = d.selected() as usize;
        if let Some(row) = LANGUAGES.get(i) {
            let mut s = state_clone.borrow_mut();
            s.language = if row.code == "auto" {
                AsrLanguage::Auto
            } else {
                AsrLanguage::Code(row.code.into())
            };
        }
    });
    dropdown
}

fn build_ptt_mode_dropdown(state: Rc<RefCell<AsrOptions>>) -> gtk::DropDown {
    let dropdown = gtk::DropDown::from_strings(&[
        "Hold (누르고 있는 동안 녹음)",
        "Toggle (한 번 누르면 시작/정지)",
    ]);
    let idx = match state.borrow().ptt_mode {
        PttMode::Hold => 0,
        PttMode::Toggle => 1,
    };
    dropdown.set_selected(idx as u32);
    let state_clone = state.clone();
    dropdown.connect_selected_notify(move |d| {
        let i = d.selected();
        state_clone.borrow_mut().ptt_mode = if i == 0 {
            PttMode::Hold
        } else {
            PttMode::Toggle
        };
    });
    dropdown
}

fn build_microphone_group(state: Rc<RefCell<AsrOptions>>) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    group.set_margin_top(8);

    let title = gtk::Label::new(Some("마이크 권한"));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    group.append(&title);

    let hint = gtk::Label::new(Some(
        "[권한 요청 및 테스트] 버튼을 누르면 짧게 마이크를 열어 \
         권한 상태를 확인합니다. Flatpak 환경에서는 시스템 다이얼로그가 표시될 수 있습니다.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    group.append(&hint);

    let button = gtk::Button::with_label("권한 요청 및 테스트");
    button.set_halign(gtk::Align::Start);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);

    {
        let state = state.clone();
        let button = button.clone();
        let status = status.clone();
        button.clone().connect_clicked(move |_| {
            button.set_sensitive(false);
            status.set_text("마이크 확인 중…");
            let state = state.clone();
            let button = button.clone();
            let status = status.clone();
            glib::MainContext::default().spawn_local(async move {
                let device = state.borrow().input_device.clone();
                let outcome = probe_microphone(device).await;
                let (msg, css_class) = match outcome {
                    MicProbeOutcome::Ok {
                        sample_rate,
                        channels,
                        captured_samples,
                    } => {
                        state.borrow_mut().mic_permission_acknowledged = true;
                        (
                            format!(
                                "마이크 접근 OK ({sample_rate} Hz, {channels} ch, 샘플 {captured_samples}개)"
                            ),
                            "success",
                        )
                    }
                    MicProbeOutcome::NoDevice => (
                        "입력 장치를 찾을 수 없습니다. 마이크가 연결되어 있는지 확인하세요.".into(),
                        "error",
                    ),
                    MicProbeOutcome::PermissionDenied { detail } => (
                        format!("권한이 거부되었습니다: {detail}"),
                        "error",
                    ),
                    MicProbeOutcome::Failed { detail } => (
                        format!("마이크 열기에 실패했습니다: {detail}"),
                        "error",
                    ),
                };
                status.remove_css_class("success");
                status.remove_css_class("error");
                status.add_css_class(css_class);
                status.set_text(&msg);
                button.set_sensitive(true);
            });
        });
    }

    group.append(&button);
    group.append(&status);
    group
}
