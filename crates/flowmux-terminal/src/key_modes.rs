// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal input mode tracking shared by terminal compatibility glue and
//! the terminal backend compatibility layer.

use std::borrow::Cow;

/// Kitty CSI-u Shift+Enter: `\x1b[13;2u`.
pub const KITTY_SHIFT_ENTER: &[u8] = b"\x1b[13;2u";

/// Legacy Shift+Enter: `ESC CR`. Agent TUIs treat this as "insert a
/// literal newline" at the prompt without submitting.
pub const LEGACY_INSERT_NEWLINE: &[u8] = b"\x1b\r";

/// Bit 0 of the Kitty keyboard flags: disambiguate escape codes.
/// When set, Shift+Enter emits `\x1b[13;2u` instead of `ESC CR`.
const KITTY_FLAG_DISAMBIGUATE: u32 = 1;

#[derive(Debug, Default, Clone)]
pub struct TerminalInputModes {
    application_cursor: bool,
    /// Current Kitty keyboard protocol flags.
    kitty_flags: u32,
    /// Stack for push/pop progressive enhancement.
    flag_stack: Vec<u32>,
    output_escape: Vec<u8>,
}

impl TerminalInputModes {
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Whether the Kitty keyboard protocol disambiguate flag is set.
    /// When enabled, Shift+Enter must emit Kitty CSI-u `\x1b[13;2u`
    /// instead of legacy `ESC CR`.
    pub fn kitty_keyboard_enabled(&self) -> bool {
        self.kitty_flags & KITTY_FLAG_DISAMBIGUATE != 0
    }

    /// Observe bytes emitted by the terminal application and update the
    /// input modes those bytes select. Tracks DECCKM (`CSI ? 1 h/l`) for
    /// cursor-key rewriting and Kitty keyboard progressive enhancement
    /// (`CSI > flags u` push, `CSI = flags ; mode u` set, `CSI < u` pop)
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
        if !self.application_cursor && !self.kitty_keyboard_enabled() {
            return Cow::Borrowed(bytes);
        }

        // Fast path: exact match for sole ESC CR (the common case for
        // Shift+Enter).
        if self.kitty_keyboard_enabled() && bytes == LEGACY_INSERT_NEWLINE {
            return Cow::Borrowed(KITTY_SHIFT_ENTER);
        }

        // When only kitty keyboard is active (no DECCKM), scan for all
        // ESC CR occurrences and rewrite them to Kitty CSI-u.
        if !self.application_cursor {
            let positions: Vec<usize> = bytes
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| (w == LEGACY_INSERT_NEWLINE).then_some(i))
                .collect();
            if positions.is_empty() {
                return Cow::Borrowed(bytes);
            }
            let expansion = KITTY_SHIFT_ENTER.len() - LEGACY_INSERT_NEWLINE.len();
            let mut out = Vec::with_capacity(bytes.len() + positions.len() * expansion);
            let mut cursor = 0;
            for pos in positions {
                out.extend_from_slice(&bytes[cursor..pos]);
                out.extend_from_slice(KITTY_SHIFT_ENTER);
                cursor = pos + LEGACY_INSERT_NEWLINE.len();
            }
            out.extend_from_slice(&bytes[cursor..]);
            return Cow::Owned(out);
        }

        let mut out = Vec::with_capacity(bytes.len());
        let mut changed = false;
        let mut i = 0;
        while i < bytes.len() {
            // Check for ESC CR (Shift+Enter legacy) first — when both
            // application-cursor and kitty keyboard are active, the
            // kitty rewrite takes priority for this specific sequence.
            if self.kitty_keyboard_enabled()
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
        if self.kitty_keyboard_enabled() {
            KITTY_SHIFT_ENTER
        } else {
            LEGACY_INSERT_NEWLINE
        }
    }

    fn apply_csi(&mut self) {
        let Some(final_byte) = self.output_escape.last().copied() else {
            return;
        };

        match final_byte {
            b'h' | b'l' => self.apply_csi_hl(final_byte),
            b'u' => self.apply_csi_kitty(),
            _ => {}
        }
    }

    /// Handle DECSET/DECRST-style CSI sequences ending in `h` or `l`.
    fn apply_csi_hl(&mut self, final_byte: u8) {
        let params = &self.output_escape[2..self.output_escape.len() - 1];
        // DEC private modes: CSI ? <num> h/l
        if let Some(private_params) = params.strip_prefix(b"?") {
            let has_decckm = private_params
                .split(|b| *b == b';')
                .any(|param| param == b"1");
            if has_decckm {
                self.application_cursor = final_byte == b'h';
            }
        }
    }

    /// Handle Kitty keyboard progressive enhancement sequences ending in `u`.
    ///
    /// Supported forms:
    /// * `CSI > flags u` — push current flags + set new flags.
    /// * `CSI = flags ; mode u` — set flags with optional mode.
    /// * `CSI < u` — pop flags from stack.
    /// * `CSI < count ; flags u` — pop `count` entries then set `flags`.
    fn apply_csi_kitty(&mut self) {
        let params = &self.output_escape[2..self.output_escape.len() - 1];
        // Push: CSI > flags u
        if let Some(flags_str) = params.strip_prefix(b">") {
            if let Some(flags) = parse_decimal_u32(flags_str) {
                self.flag_stack.push(self.kitty_flags);
                self.kitty_flags = flags;
            }
            return;
        }

        // Set: CSI = flags ; mode u
        if let Some(eq_params) = params.strip_prefix(b"=") {
            let mut parts = eq_params.split(|b| *b == b';');
            if let Some(flags_bytes) = parts.next() {
                if let Some(flags) = parse_decimal_u32(flags_bytes) {
                    self.kitty_flags = flags;
                }
            }
            // mode (second parameter) is parsed but not used for
            // Shift+Enter rewriting; all modes set the flags field.
            return;
        }

        // Pop: CSI < [count] [; flags] u
        if let Some(pop_params) = params.strip_prefix(b"<") {
            if pop_params.is_empty() {
                // Simple pop: CSI < u — restore one saved state
                if let Some(prev) = self.flag_stack.pop() {
                    self.kitty_flags = prev;
                }
                return;
            }
            let mut parts = pop_params.split(|b| *b == b';');
            let first = parts.next().unwrap_or(b"");
            let second = parts.next();

            if second.is_none() && !first.is_empty() {
                // CSI < count u — pop count times, each restores flags
                if let Some(count) = parse_decimal_u32(first) {
                    for _ in 0..count {
                        if let Some(prev) = self.flag_stack.pop() {
                            self.kitty_flags = prev;
                        } else {
                            break;
                        }
                    }
                }
            } else {
                // CSI < count ; flags u — pop count (discard), then set flags
                let count = parse_decimal_u32(first).unwrap_or(1);
                for _ in 0..count {
                    if self.flag_stack.pop().is_none() {
                        break;
                    }
                }
                if let Some(flags) = second.and_then(parse_decimal_u32) {
                    self.kitty_flags = flags;
                } else if let Some(&top) = self.flag_stack.last() {
                    self.kitty_flags = top;
                }
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

/// Parse a decimal `u32` from a byte slice. Returns `None` on empty input,
/// non-digit bytes, or overflow.
fn parse_decimal_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
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

    // ---- Kitty keyboard progressive enhancement tests ----

    #[test]
    fn kitty_push_enables_disambiguate() {
        let mut modes = TerminalInputModes::default();
        // CSI > 1 u — push, set flag 1 (disambiguate)
        modes.observe_output(b"\x1b[>1u");

        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_push_stacks_and_pop_restores() {
        let mut modes = TerminalInputModes::default();
        // Push flag 1 → disambiguate on
        modes.observe_output(b"\x1b[>1u");
        assert!(modes.kitty_keyboard_enabled());

        // Push flag 3 (disambiguate + report event types) → still on
        modes.observe_output(b"\x1b[>3u");
        assert!(modes.kitty_keyboard_enabled());

        // Pop: CSI < u — restore flag 1 → still on
        modes.observe_output(b"\x1b[<u");
        assert!(modes.kitty_keyboard_enabled());

        // Pop again — restore default (0) → disambiguate off
        modes.observe_output(b"\x1b[<u");
        assert!(!modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }

    #[test]
    fn kitty_set_with_mode() {
        let mut modes = TerminalInputModes::default();
        // CSI = 1 ; 1 u — set flags=1 with mode=1 (progressive enhancement)
        modes.observe_output(b"\x1b[=1;1u");
        assert!(modes.kitty_keyboard_enabled());
    }

    #[test]
    fn kitty_set_zero_disables() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");
        assert!(modes.kitty_keyboard_enabled());

        // CSI = 0 ; 0 u — reset to defaults
        modes.observe_output(b"\x1b[=0;0u");
        assert!(!modes.kitty_keyboard_enabled());
    }

    #[test]
    fn kitty_pop_with_count_and_flags() {
        let mut modes = TerminalInputModes::default();
        // Push three levels: 1, 2, 4
        modes.observe_output(b"\x1b[>1u");
        modes.observe_output(b"\x1b[>2u");
        modes.observe_output(b"\x1b[>4u");
        // Flag 4 does NOT include bit 0, so disambiguate is off
        assert!(!modes.kitty_keyboard_enabled());

        // CSI < 2 ; 1 u — pop 2 entries, then set flags=1
        modes.observe_output(b"\x1b[<2;1u");
        // After popping 2, we're at flags=1 → disambiguate on
        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_pop_empty_stack_is_noop() {
        let mut modes = TerminalInputModes::default();
        // Pop on empty stack: should not panic, flags stay 0
        modes.observe_output(b"\x1b[<u");
        assert!(!modes.kitty_keyboard_enabled());
    }

    #[test]
    fn kitty_push_pop_split_chunks() {
        let mut modes = TerminalInputModes::default();
        // Chunk the push across multiple observe_output calls
        modes.observe_output(b"\x1b[>");
        modes.observe_output(b"1");
        modes.observe_output(b"u");

        assert!(modes.kitty_keyboard_enabled());

        // Chunk the pop
        modes.observe_output(b"\x1b[<");
        modes.observe_output(b"u");

        assert!(!modes.kitty_keyboard_enabled());
    }

    #[test]
    fn kitty_multiple_shift_enter_in_one_input() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");

        // Two ESC CR sequences in one buffer
        let input = b"\x1b\r\x1b\r";
        let rewritten = modes.rewrite_input(input);
        // Each ESC CR (2 bytes) → KITTY_SHIFT_ENTER (7 bytes)
        assert_eq!(rewritten.as_ref().len(), 14);
        assert_eq!(&rewritten.as_ref()[0..7], KITTY_SHIFT_ENTER);
        assert_eq!(&rewritten.as_ref()[7..14], KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_multiple_shift_enter_with_text_between() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");

        let input = b"a\x1b\rb\x1b\rc";
        let rewritten = modes.rewrite_input(input);
        let out = rewritten.as_ref();
        // Expected: a + KITTY_SHIFT_ENTER + b + KITTY_SHIFT_ENTER + c
        assert_eq!(out.len(), 1 + 7 + 1 + 7 + 1);
        assert_eq!(out[0], b'a');
        assert_eq!(&out[1..8], KITTY_SHIFT_ENTER);
        assert_eq!(out[8], b'b');
        assert_eq!(&out[9..16], KITTY_SHIFT_ENTER);
        assert_eq!(out[16], b'c');
    }

    #[test]
    fn kitty_and_decckm_can_both_be_active() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?1h"); // enable DECCKM
        modes.observe_output(b"\x1b[>1u"); // enable Kitty disambiguate

        assert!(modes.application_cursor());
        assert!(modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), KITTY_SHIFT_ENTER);

        // Cursor keys still rewritten for DECCKM
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1bOA");
    }

    #[test]
    fn kitty_shift_enter_rewrite() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");

        // Shift+Enter (ESC CR) rewritten to Kitty CSI-u
        assert_eq!(modes.rewrite_input(b"\x1b\r").as_ref(), KITTY_SHIFT_ENTER);
    }

    #[test]
    fn kitty_other_input_unchanged() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");

        // Plain Enter unchanged
        assert_eq!(modes.rewrite_input(b"\r").as_ref(), b"\r");
        // Ctrl+C unchanged
        assert_eq!(modes.rewrite_input(b"\x03").as_ref(), b"\x03");
        // Normal arrow keys unchanged (no DECCKM active)
        assert_eq!(modes.rewrite_input(b"\x1b[A").as_ref(), b"\x1b[A");
    }

    #[test]
    fn kitty_shift_enter_in_mixed_chunk() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[>1u");

        // ESC CR preceded/followed by plain text
        let input = b"hello\x1b\rworld";
        let rewritten = modes.rewrite_input(input);
        assert!(rewritten.as_ref().len() > input.len());
        assert!(
            rewritten
                .as_ref()
                .windows(KITTY_SHIFT_ENTER.len())
                .any(|w| w == KITTY_SHIFT_ENTER),
            "KITTY_SHIFT_ENTER not found in rewritten output"
        );
        assert!(rewritten.as_ref().starts_with(b"hello"));
        assert!(rewritten.as_ref().ends_with(b"world"));
    }

    #[test]
    fn kitty_disambiguate_off_no_rewrite() {
        let mut modes = TerminalInputModes::default();
        // Flag 2 (report event types) but not flag 1 (disambiguate)
        modes.observe_output(b"\x1b[>2u");

        assert!(!modes.kitty_keyboard_enabled());
        // ESC CR not rewritten
        assert_eq!(modes.rewrite_input(b"\x1b\r").as_ref(), b"\x1b\r");
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }

    #[test]
    fn kitty_shift_enter_bytes_constant_is_correct() {
        assert_eq!(KITTY_SHIFT_ENTER, b"\x1b[13;2u");
        assert_eq!(LEGACY_INSERT_NEWLINE, b"\x1b\r");
    }

    #[test]
    fn unrelated_csi_does_not_affect_kitty_keyboard() {
        let mut modes = TerminalInputModes::default();
        modes.observe_output(b"\x1b[?25l");
        modes.observe_output(b"\x1b[?25h");
        modes.observe_output(b"\x1b[0m");

        assert!(!modes.kitty_keyboard_enabled());
        assert_eq!(modes.shift_enter_bytes(), LEGACY_INSERT_NEWLINE);
    }

    #[test]
    fn parse_decimal_u32_valid() {
        assert_eq!(parse_decimal_u32(b"0"), Some(0));
        assert_eq!(parse_decimal_u32(b"1"), Some(1));
        assert_eq!(parse_decimal_u32(b"27127"), Some(27127));
        assert_eq!(parse_decimal_u32(b"4294967295"), Some(u32::MAX));
    }

    #[test]
    fn parse_decimal_u32_invalid() {
        assert_eq!(parse_decimal_u32(b""), None);
        assert_eq!(parse_decimal_u32(b"-1"), None);
        assert_eq!(parse_decimal_u32(b"1a"), None);
        // Overflow (> u32::MAX)
        assert_eq!(parse_decimal_u32(b"4294967296"), None);
    }

    #[test]
    fn kitty_pop_with_count_only() {
        let mut modes = TerminalInputModes::default();
        // Push 1, 3, 7 — only 1 and 3 have disambiguate bit
        modes.observe_output(b"\x1b[>1u");
        modes.observe_output(b"\x1b[>3u");
        modes.observe_output(b"\x1b[>7u");
        // Flag 7 has bit 0 set → disambiguate on
        assert!(modes.kitty_keyboard_enabled());

        // CSI < 2 u — pop 2 entries, restoring flags=1
        modes.observe_output(b"\x1b[<2u");
        assert!(modes.kitty_keyboard_enabled());
    }

    #[test]
    fn kitty_set_without_mode() {
        let mut modes = TerminalInputModes::default();
        // CSI = 1 u — set flags=1, no explicit mode
        modes.observe_output(b"\x1b[=1u");
        assert!(modes.kitty_keyboard_enabled());
    }
}
