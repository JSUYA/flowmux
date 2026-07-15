// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal input mode tracking shared by terminal compatibility glue and
//! the terminal backend compatibility layer.

use std::borrow::Cow;

/// Kitty CSI-u Shift+Enter: `\x1b[13;2u`.
pub const KITTY_SHIFT_ENTER: &[u8] = b"\x1b[13;2u";

/// Legacy Shift+Enter: `ESC CR`. Agent TUIs treat this as "insert a
/// literal newline" at the prompt without submitting.
pub const LEGACY_INSERT_NEWLINE: &[u8] = b"\x1b\r";

#[derive(Debug, Default, Clone)]
pub struct TerminalInputModes {
    application_cursor: bool,
    kitty_keyboard_enabled: bool,
    output_escape: Vec<u8>,
}

impl TerminalInputModes {
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Whether the foreground application has enabled the Kitty
    /// keyboard protocol (`CSI > 27127 h`). When enabled, Shift+Enter
    /// must emit Kitty CSI-u `\x1b[13;2u` instead of legacy `ESC CR`.
    pub fn kitty_keyboard_enabled(&self) -> bool {
        self.kitty_keyboard_enabled
    }

    /// Observe bytes emitted by the terminal application and update the
    /// input modes those bytes select. Tracks DECCKM (`CSI ? 1 h/l`) for
    /// cursor-key rewriting and Kitty keyboard protocol (`CSI > 27127 h/l`)
    /// for Pi-compatible Shift+Enter.
    pub fn observe_output(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.output_escape.is_empty() {
                if byte == 0x1b {
                    self.output_escape.push(byte);
                }
                continue;
            }

            self.output_escape.push(byte);
            if self.output_escape.len() > 32 {
                self.output_escape.clear();
                continue;
            }

            if self.output_escape == b"\x1b=" || self.output_escape == b"\x1b>" {
                self.output_escape.clear();
                continue;
            }

            if self.output_escape.len() > 2
                && self.output_escape.starts_with(b"\x1b[")
                && (0x40..=0x7e).contains(&byte)
            {
                self.apply_csi();
                self.output_escape.clear();
            } else if !self.output_escape.starts_with(b"\x1b[") && self.output_escape.len() >= 2 {
                self.output_escape.clear();
            }
        }
    }

    /// Rewrite input bytes to match the foreground app's terminal mode.
    ///
    /// * Cursor keys are rewritten to application-cursor form when DECCKM is
    ///   active.
    /// * `ESC CR` (Shift+Enter legacy form) is rewritten to Kitty CSI-u
    ///   `\x1b[13;2u` when the Kitty keyboard protocol is enabled.
    pub fn rewrite_input<'a>(&self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        if !self.application_cursor && !self.kitty_keyboard_enabled {
            return Cow::Borrowed(bytes);
        }

        // Fast path: exact match for sole ESC CR (the common case for
        // Shift+Enter).
        if self.kitty_keyboard_enabled && bytes == LEGACY_INSERT_NEWLINE {
            return Cow::Borrowed(KITTY_SHIFT_ENTER);
        }

        // When only kitty keyboard is active (no DECCKM), scan for ESC CR
        // and rewrite to Kitty CSI-u. No cursor-key rewriting needed.
        if !self.application_cursor {
            if let Some(pos) = find_legacy_shift_enter(bytes) {
                let mut out = Vec::with_capacity(bytes.len() + KITTY_SHIFT_ENTER.len() - 2);
                out.extend_from_slice(&bytes[..pos]);
                out.extend_from_slice(KITTY_SHIFT_ENTER);
                out.extend_from_slice(&bytes[pos + 2..]);
                return Cow::Owned(out);
            }
            return Cow::Borrowed(bytes);
        }

        let mut out = Vec::with_capacity(bytes.len());
        let mut changed = false;
        let mut i = 0;
        while i < bytes.len() {
            // Check for ESC CR (Shift+Enter legacy) first — when both
            // application-cursor and kitty keyboard are active, the
            // kitty rewrite takes priority for this specific sequence.
            if self.kitty_keyboard_enabled
                && i + 1 < bytes.len()
                && bytes[i] == 0x1b
                && bytes[i + 1] == b'\r'
            {
                out.extend_from_slice(KITTY_SHIFT_ENTER);
                changed = true;
                i += 2;
                continue;
            }
            if i + 2 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
                if let Some(final_byte) = app_cursor_final(bytes[i + 2]) {
                    out.extend_from_slice(&[0x1b, b'O', final_byte]);
                    changed = true;
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }

        if changed {
            Cow::Owned(out)
        } else {
            Cow::Borrowed(bytes)
        }
    }

    /// Resolve the Shift+Enter byte sequence for the current terminal mode.
    ///
    /// Returns Kitty CSI-u `\x1b[13;2u` when the foreground app has enabled
    /// the Kitty keyboard protocol, or legacy `ESC CR` otherwise.
    pub fn shift_enter_bytes(&self) -> &'static [u8] {
        if self.kitty_keyboard_enabled {
            KITTY_SHIFT_ENTER
        } else {
            LEGACY_INSERT_NEWLINE
        }
    }

    fn apply_csi(&mut self) {
        let Some(final_byte) = self.output_escape.last().copied() else {
            return;
        };
        if final_byte != b'h' && final_byte != b'l' {
            return;
        }
        let params = &self.output_escape[2..self.output_escape.len() - 1];

        // DEC private modes: CSI ? <num> h/l
        if let Some(private_params) = params.strip_prefix(b"?") {
            let has_decckm = private_params
                .split(|b| *b == b';')
                .any(|param| param == b"1");
            if has_decckm {
                self.application_cursor = final_byte == b'h';
            }
            return;
        }

        // Kitty keyboard protocol: CSI > 27127 h/l
        if let Some(kitty_params) = params.strip_prefix(b">") {
            let has_kitty = kitty_params
                .split(|b| *b == b';')
                .any(|param| param == b"27127");
            if has_kitty {
                self.kitty_keyboard_enabled = final_byte == b'h';
            }
        }
    }
}

fn app_cursor_final(final_byte: u8) -> Option<u8> {
    match final_byte {
        b'A' | b'B' | b'C' | b'D' | b'H' | b'F' => Some(final_byte),
        _ => None,
    }
}

/// Find the position of the legacy Shift+Enter sequence (`ESC CR`) in `bytes`.
fn find_legacy_shift_enter(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == LEGACY_INSERT_NEWLINE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cursor_keys_stay_in_normal_mode() {
        let modes = TerminalInputModes::default();

        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1b[A");
        assert_eq!(modes.rewrite_input(b"\x1b[B").as_ref(), b"\x1b[B");
        assert!(!modes.application_cursor());
        assert!(!modes.kitty_keyboard_enabled());
    }

    #[test]
    fn default_shift_enter_is_legacy_esc_cr() {
        let modes = TerminalInputModes::default();
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }

    #[test]
    fn smkx_application_cursor_mode_rewrites_tig_arrow_keys() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?1h\x1b=");

        assert!(modes.application_cursor());
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1bOA");
        assert_eq!(modes.rewrite_input(b"\x1b[B").as_ref(), b"\x1bOB");
        assert_eq!(modes.rewrite_input(b"\x1b[C").as_ref(), b"\x1bOC");
        assert_eq!(modes.rewrite_input(b"\x1b[D").as_ref(), b"\x1bOD");
    }

    #[test]
    fn rmkx_restores_normal_cursor_mode() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?1h\x1b=");
        modes.observe_output(b"\x1b[?1l\x1b>");

        assert!(!modes.application_cursor());
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1b[A");
        assert_eq!(modes.rewrite_input(b"\x1b[B").as_ref(), b"\x1b[B");
    }

    #[test]
    fn decckm_tracking_survives_split_output_chunks() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?");
        modes.observe_output(b"1h");

        assert!(modes.application_cursor());
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1bOA");
    }

    #[test]
    fn non_cursor_input_is_preserved_in_application_cursor_mode() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?1h");

        assert_eq!(modes.rewrite_input(b"abc\r\x1b").as_ref(), b"abc\r\x1b");
        assert_eq!(modes.rewrite_input(b"\x1b[3~").as_ref(), b"\x1b[3~");
        assert_eq!(modes.rewrite_input(b"\x1bOP").as_ref(), b"\x1bOP");
    }

    #[test]
    fn private_csi_list_updates_when_decckm_is_present() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?7;1h");

        assert!(modes.application_cursor());
    }

    // ---- Kitty keyboard protocol tests ----

    #[test]
    fn kitty_keyboard_protocol_enable_detected() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>27127h");

        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_keyboard_protocol_disable_detected() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>27127h");
        assert!(modes.kitty_keyboard_enabled());

        modes.observe_output(b"\x1b[>27127l");
        assert!(!modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }

    #[test]
    fn kitty_keyboard_and_decckm_can_both_be_active() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?1h"); // enable DECCKM
        modes.observe_output(b"\x1b[>27127h"); // enable Kitty keyboard

        assert!(modes.application_cursor());
        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);

        // Cursor keys still rewritten for DECCKM
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1bOA");
    }

    #[test]
    fn kitty_keyboard_shift_enter_rewrite() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>27127h");

        // Shift+Enter (ESC CR) rewritten to Kitty CSI-u
        assert_eq!(modes.rewrite_input(b"\x1b\r").as_ref(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_keyboard_other_input_unchanged() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>27127h");

        // Plain Enter unchanged
        assert_eq!(modes.rewrite_input(b"\r").as_ref(), b"\r");
        // Ctrl+C unchanged
        assert_eq!(modes.rewrite_input(b"\x03").as_ref(), b"\x03");
        // Normal arrow keys unchanged (no DECCKM active)
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1b[A");
    }

    #[test]
    fn kitty_keyboard_shift_enter_in_mixed_chunk() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>27127h");

        // ESC CR followed by plain text (unlikely but must be handled)
        let input = b"hello\x1b\rworld";
        let rewritten = modes.rewrite_input(input);
        // The ESC CR (2 bytes) should be replaced by KITTY_SHIFT_ENTER (7 bytes)
        assert!(rewritten.as_ref().len() > input.len());
        assert!(
            rewritten
                .as_ref()
                .windows(KITTY_SHIFT_ENTER.len())
                .any(|w| w == KITTY_SHIFT_ENTER),
            "KITTY_SHIFT_ENTER not found in rewritten output"
        );
        // The surrounding text should be preserved
        assert!(rewritten.as_ref().starts_with(b"hello"));
        assert!(rewritten.as_ref().ends_with(b"world"));
    }

    #[test]
    fn kitty_keyboard_split_chunks() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>");
        modes.observe_output(b"27127h");

        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_shift_enter_bytes_constant_is_correct() {
        // CSI 13 ; 2 u
        assert_eq!(KITTY_SHIFT_ENTER, b"\x1b[13;2u");
        // Legacy fallback
        assert_eq!(LEGACY_INSERT_NEWLINE, b"\x1b\r");
    }

    #[test]
    fn unrelated_csi_does_not_affect_kitty_keyboard() {
        let mut modes = TerminalInputModes::default();
        // Terminal reset, cursor visibility, etc.
        modes.observe_output(b"\x1b[?25l");
        modes.observe_output(b"\x1b[?25h");
        modes.observe_output(b"\x1b[0m");

        // Kitty keyboard is not affected by unrelated CSIs
        assert!(!modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }
}
