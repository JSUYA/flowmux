//! Read-only loader for `~/.config/ghostty/config`.
//!
//! The Ghostty config file is documented as `key = value` lines with `#`
//! comments and `key = value, value` for lists. We only extract the
//! subset flowmux needs (font, theme, colors); unknown keys are kept as
//! raw strings in `extras` so the data round-trips for diagnostics.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct GhosttyConfig {
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub theme: Option<String>,
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub cursor_color: Option<String>,
    pub extras: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub fn load(path: &Path) -> Result<GhosttyConfig, LoadError> {
    let text = std::fs::read_to_string(path)?;
    Ok(parse(&text))
}

/// Strip an inline trailing comment introduced by whitespace + `#`.
/// Hex colors (`#1e1e2e`) and similar values that start with `#` are
/// preserved because they are not preceded by whitespace within the
/// value substring.
fn strip_inline_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i > 0 && bytes[i - 1].is_ascii_whitespace() {
            return value[..i].trim_end();
        }
        i += 1;
    }
    value
}

pub fn parse(text: &str) -> GhosttyConfig {
    let mut cfg = GhosttyConfig::default();
    for raw in text.lines() {
        let line = raw.trim();
        // Ghostty's config format treats `#` at the start of a (trimmed)
        // line as a comment. We deliberately do NOT split on inline `#`
        // since values like hex colors (`#1e1e2e`) start with `#`.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let value = strip_inline_comment(v.trim());
        match key {
            "font-family" => cfg.font_family = Some(value.into()),
            "font-size" => cfg.font_size = value.parse().ok(),
            "theme" => cfg.theme = Some(value.into()),
            "background" => cfg.background = Some(value.into()),
            "foreground" => cfg.foreground = Some(value.into()),
            "cursor-color" => cfg.cursor_color = Some(value.into()),
            other => {
                cfg.extras.insert(other.into(), value.into());
            }
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_ghostty_config() {
        let raw = "\
            # Comment line\n\
            font-family = JetBrains Mono\n\
            font-size = 13\n\
            theme = catppuccin-mocha\n\
            background = #1e1e2e   # inline comment\n\
            keybind = ctrl+s>r=reload_config\n\
        ";
        let cfg = parse(raw);
        assert_eq!(cfg.font_family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(cfg.font_size, Some(13.0));
        assert_eq!(cfg.theme.as_deref(), Some("catppuccin-mocha"));
        assert_eq!(cfg.background.as_deref(), Some("#1e1e2e"));
        assert!(cfg.extras.contains_key("keybind"));
    }
}
