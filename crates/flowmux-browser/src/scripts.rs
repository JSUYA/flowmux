// SPDX-License-Identifier: GPL-3.0-or-later
//! JavaScript snippets the controller injects into the page to
//! implement snapshots, refs, clicks, fills, etc.
//!
//! Each builder returns a string ready to hand to
//! `WebView::evaluate_javascript`. The values they evaluate to are
//! always either:
//!
//!   * a JSON string the controller `serde_json::from_str`-decodes
//!     ([`SNAPSHOT_JS`]), or
//!   * the literal string `"ok"` on success / `"error: <reason>"` on
//!     a soft failure (e.g. ref not in DOM).
//!
//! The controller treats `"ok"` as success and any non-`"ok"` value
//! as [`crate::BrowserError::RefNotFound`] / `Eval`. Keeping the
//! return shape uniform makes the controller's WebKit glue trivial.

/// Walk the document for everything an agent might want to act on
/// — links, buttons, inputs, headings, anything with an explicit
/// role — and emit a flat JSON snapshot. Each visible element gets a
/// stable `data-flowmux-ref="eN"` attribute so subsequent
/// `clickByRef` / `fillByRef` calls can find it without depending on
/// the page's own selectors.
pub const SNAPSHOT_JS: &str = r#"
(function() {
  const out = [];
  let counter = 0;
  function visible(el) {
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4) return false;
    const cs = window.getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') return false;
    if (Number(cs.opacity) === 0) return false;
    return true;
  }
  function name(el) {
    return (
      el.getAttribute('aria-label') ||
      el.getAttribute('alt') ||
      el.getAttribute('title') ||
      el.getAttribute('placeholder') ||
      (el.innerText || '').trim().slice(0, 120)
    );
  }
  document.querySelectorAll(
    'a,button,input,textarea,select,[role],h1,h2,h3,label,summary'
  ).forEach((el) => {
    if (!visible(el)) return;
    const r = el.getBoundingClientRect();
    counter += 1;
    const ref = 'e' + counter;
    el.setAttribute('data-flowmux-ref', ref);
    out.push({
      ref,
      tag: el.tagName.toLowerCase(),
      role: el.getAttribute('role') || el.tagName.toLowerCase(),
      name: name(el),
      bbox: [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)],
    });
  });
  return JSON.stringify({ url: location.href, title: document.title, nodes: out });
})()
"#;

/// Click the element previously stamped with `data-flowmux-ref=<r>`.
/// Returns `"ok"` on success, `"error: ref not found"` otherwise.
pub fn click_by_ref(r: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            el.click();
            return "ok";
        }})()"#,
        r = js_string(r)
    )
}

/// Set `value` on an input/textarea (`<select>` should use
/// [`select_option_by_ref`] instead) and dispatch the standard
/// `input` + `change` events so framework listeners fire.
pub fn fill_by_ref(r: &str, value: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            const setter = Object.getOwnPropertyDescriptor(el.__proto__, 'value');
            if (setter && setter.set) {{
                setter.set.call(el, "{v}");
            }} else {{
                el.value = "{v}";
            }}
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return "ok";
        }})()"#,
        r = js_string(r),
        v = js_string(value)
    )
}

/// `<select>` value picker — looks up an `<option>` by its `value`
/// or, failing that, by its visible text.
pub fn select_option_by_ref(r: &str, value: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            const want = "{v}";
            for (const opt of el.options) {{
                if (opt.value === want || opt.textContent.trim() === want) {{
                    opt.selected = true;
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    return "ok";
                }}
            }}
            return "error: option not found";
        }})()"#,
        r = js_string(r),
        v = js_string(value)
    )
}

/// Scroll the element matched by ref into view, with a sub-pixel
/// offset applied to the body afterwards. Two coordinates so callers
/// can scroll a list-pane plus its container.
pub fn scroll_by_ref(r: &str, x: i32, y: i32) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            el.scrollIntoView({{ block: "center", inline: "nearest" }});
            window.scrollBy({x}, {y});
            return "ok";
        }})()"#,
        r = js_string(r),
        x = x,
        y = y
    )
}

/// Read element's `innerText`.
pub fn text_of(r: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            return (el.innerText || "").toString();
        }})()"#,
        r = js_string(r)
    )
}

/// Read an input/textarea/select's `value`.
pub fn value_of(r: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            return (el.value || "").toString();
        }})()"#,
        r = js_string(r)
    )
}

/// Read an arbitrary attribute. Returns the empty string if the
/// element exists but the attribute does not (matches DOM behavior).
pub fn attr_of(r: &str, name: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.querySelector('[data-flowmux-ref="{r}"]');
            if (!el) return "error: ref not found";
            return (el.getAttribute("{n}") || "").toString();
        }})()"#,
        r = js_string(r),
        n = js_string(name)
    )
}

/// Send each character of `text` as a `keydown`+`input`+`keyup`
/// triple to the active element. Mirrors what a user typing into a
/// focused input would produce.
pub fn type_keys(text: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.activeElement;
            if (!el) return "error: no focus";
            const text = "{t}";
            for (const ch of text) {{
                el.dispatchEvent(new KeyboardEvent('keydown', {{ key: ch, bubbles: true }}));
                if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {{
                    el.value += ch;
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                }}
                el.dispatchEvent(new KeyboardEvent('keyup', {{ key: ch, bubbles: true }}));
            }}
            if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {{
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            }}
            return "ok";
        }})()"#,
        t = js_string(text)
    )
}

/// Send a single named key (`Enter`, `Tab`, `ArrowDown`, …) as a
/// `keydown`+`keyup` pair to the active element.
pub fn press_key(key: &str) -> String {
    format!(
        r#"(function() {{
            const el = document.activeElement || document.body;
            const k = "{k}";
            el.dispatchEvent(new KeyboardEvent('keydown', {{ key: k, bubbles: true }}));
            el.dispatchEvent(new KeyboardEvent('keyup', {{ key: k, bubbles: true }}));
            return "ok";
        }})()"#,
        k = js_string(key)
    )
}

/// Conservative JS string escaper — covers the cases the agent
/// surfaces actually pass us (URLs, names, free text). Doesn't try
/// to be a general-purpose JS escaper, but is safe for what we use.
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quick sanity check: every script should be a self-invoking
    /// IIFE and have balanced parens / braces. The controller
    /// expects a single expression that returns a string.
    fn assert_balanced(js: &str) {
        let mut paren = 0i32;
        let mut brace = 0i32;
        let mut bracket = 0i32;
        let mut in_str = None::<char>;
        let mut prev = '\0';
        for c in js.chars() {
            if let Some(q) = in_str {
                if c == q && prev != '\\' {
                    in_str = None;
                }
            } else {
                match c {
                    '"' | '\'' | '`' => in_str = Some(c),
                    '(' => paren += 1,
                    ')' => paren -= 1,
                    '{' => brace += 1,
                    '}' => brace -= 1,
                    '[' => bracket += 1,
                    ']' => bracket -= 1,
                    _ => {}
                }
            }
            prev = c;
        }
        assert_eq!(paren, 0, "unbalanced parens in:\n{js}");
        assert_eq!(brace, 0, "unbalanced braces in:\n{js}");
        assert_eq!(bracket, 0, "unbalanced brackets in:\n{js}");
    }

    #[test]
    fn snapshot_js_is_balanced_iife() {
        assert!(SNAPSHOT_JS.trim_start().starts_with("(function()"));
        assert_balanced(SNAPSHOT_JS);
    }

    #[test]
    fn click_by_ref_embeds_ref() {
        let s = click_by_ref("e7");
        assert!(s.contains(r#"data-flowmux-ref="e7""#));
        assert!(s.contains("el.click()"));
        assert_balanced(&s);
    }

    #[test]
    fn fill_by_ref_dispatches_input_and_change() {
        let s = fill_by_ref("e1", "user@example.com");
        assert!(s.contains("'input'"));
        assert!(s.contains("'change'"));
        assert!(s.contains("user@example.com"));
        assert_balanced(&s);
    }

    #[test]
    fn fill_by_ref_escapes_quote_in_value() {
        let s = fill_by_ref("e1", r#"O'Reilly"#);
        // The unescaped value would break out of the string literal
        // around `el.value = "..."`. After escaping the literal `'`
        // stays unchanged but the surrounding `"..."` is balanced.
        assert!(s.contains("O'Reilly"));
        assert_balanced(&s);
    }

    #[test]
    fn fill_by_ref_escapes_double_quote_in_value() {
        let s = fill_by_ref("e1", r#"say "hi""#);
        assert!(s.contains(r#"\"hi\""#));
        assert_balanced(&s);
    }

    #[test]
    fn select_option_by_ref_balanced() {
        assert_balanced(&select_option_by_ref("e1", "USD"));
    }

    #[test]
    fn scroll_by_ref_inlines_coords() {
        let s = scroll_by_ref("e1", 10, -5);
        assert!(s.contains("scrollBy(10, -5)"));
        assert_balanced(&s);
    }

    #[test]
    fn read_helpers_balanced() {
        assert_balanced(&text_of("e1"));
        assert_balanced(&value_of("e1"));
        assert_balanced(&attr_of("e1", "href"));
    }

    #[test]
    fn type_keys_escapes_newline() {
        let s = type_keys("a\nb");
        assert!(s.contains(r"\n"));
        assert_balanced(&s);
    }

    #[test]
    fn press_key_inlines_name() {
        let s = press_key("Enter");
        assert!(s.contains("\"Enter\""));
        assert_balanced(&s);
    }

    #[test]
    fn js_string_escapes_known_specials() {
        assert_eq!(js_string(r#"a\b"#), "a\\\\b");
        assert_eq!(js_string("a\nb"), "a\\nb");
        assert_eq!(js_string("a\tb"), "a\\tb");
        assert_eq!(js_string("a\rb"), "a\\rb");
        assert_eq!(js_string("\"hi\""), "\\\"hi\\\"");
    }

    #[test]
    fn js_string_passes_safe_ascii_through() {
        assert_eq!(js_string("hello world"), "hello world");
        assert_eq!(js_string("a-b_c.d"), "a-b_c.d");
    }

    #[test]
    fn js_string_escapes_low_control_chars() {
        let s = js_string("\u{0001}");
        assert_eq!(s, "\\u0001");
    }

    #[test]
    fn js_string_escapes_line_separators() {
        // U+2028 / U+2029 break JS string literals if not escaped.
        assert_eq!(js_string("\u{2028}"), "\\u2028");
        assert_eq!(js_string("\u{2029}"), "\\u2029");
    }
}
