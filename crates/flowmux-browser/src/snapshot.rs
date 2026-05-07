// SPDX-License-Identifier: GPL-3.0-or-later
//! Serializable shape of the page snapshot the controller produces.
//!
//! The snapshot is built by [`crate::scripts::SNAPSHOT_JS`] running
//! inside the WebView. The script returns a JSON string with this
//! exact shape; the controller parses it with `serde_json` and hands
//! it to callers (CLI / agent automation) as a [`DomSnapshot`].

use serde::{Deserialize, Serialize};

/// Top-level snapshot: page metadata + flat list of interactable
/// nodes. The list is *flat by design* — agents don't need the full
/// DOM tree, they need the leaves they can click / read / fill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomSnapshot {
    pub url: String,
    pub title: String,
    pub nodes: Vec<DomNode>,
}

/// A single node in the snapshot.
///
/// `r#ref` is a per-snapshot identifier ("e1", "e2", …) the script
/// stamps onto the live DOM as the `data-flowmux-ref` attribute, so
/// follow-up calls (`click`, `fill`, `value_of`, …) can find the
/// element again without depending on the page's own selectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomNode {
    /// Stable per-snapshot reference id, e.g. `"e1"`.
    pub r#ref: String,
    /// Lowercased HTML tag — `"a"`, `"button"`, `"input"`, …
    pub tag: String,
    /// ARIA role if explicit, otherwise the tag.
    pub role: String,
    /// Best-effort accessible name (aria-label, alt, title,
    /// placeholder, or trimmed innerText).
    pub name: String,
    /// `[left, top, width, height]` in CSS pixels relative to the
    /// viewport.
    pub bbox: [i32; 4],
}

impl DomSnapshot {
    pub fn empty(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: String::new(),
            nodes: Vec::new(),
        }
    }

    /// Look up a node by its `ref` id. Used by tests and by the
    /// agent-browser CLI verbs that take a ref string from the user.
    pub fn find(&self, r: &str) -> Option<&DomNode> {
        self.nodes.iter().find(|n| n.r#ref == r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DomSnapshot {
        DomSnapshot {
            url: "https://example.com/login".into(),
            title: "Sign in".into(),
            nodes: vec![
                DomNode {
                    r#ref: "e1".into(),
                    tag: "input".into(),
                    role: "textbox".into(),
                    name: "Email".into(),
                    bbox: [40, 200, 320, 36],
                },
                DomNode {
                    r#ref: "e2".into(),
                    tag: "button".into(),
                    role: "button".into(),
                    name: "Sign in".into(),
                    bbox: [40, 280, 120, 36],
                },
            ],
        }
    }

    #[test]
    fn snapshot_serde_roundtrips() {
        let s = sample();
        let json = serde_json::to_string(&s).unwrap();
        let back: DomSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn snapshot_uses_ref_field_name_in_json() {
        let s = sample();
        let json = serde_json::to_string(&s).unwrap();
        // The JS-side script writes `ref`, so the JSON must contain
        // it as a key (not the Rust-only `r#ref`).
        assert!(json.contains("\"ref\":\"e1\""));
        assert!(json.contains("\"ref\":\"e2\""));
    }

    #[test]
    fn snapshot_parses_minimal_payload() {
        let raw = r#"{"url":"about:blank","title":"","nodes":[]}"#;
        let s: DomSnapshot = serde_json::from_str(raw).unwrap();
        assert_eq!(s.url, "about:blank");
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn snapshot_parses_real_world_node() {
        let raw = r#"{
            "url":"https://x.test/",
            "title":"X",
            "nodes":[
                {"ref":"e1","tag":"a","role":"link","name":"Home","bbox":[10,20,30,40]}
            ]
        }"#;
        let s: DomSnapshot = serde_json::from_str(raw).unwrap();
        assert_eq!(s.nodes.len(), 1);
        let n = &s.nodes[0];
        assert_eq!(n.r#ref, "e1");
        assert_eq!(n.tag, "a");
        assert_eq!(n.role, "link");
        assert_eq!(n.name, "Home");
        assert_eq!(n.bbox, [10, 20, 30, 40]);
    }

    #[test]
    fn empty_constructor_works() {
        let s = DomSnapshot::empty("about:blank");
        assert_eq!(s.url, "about:blank");
        assert_eq!(s.title, "");
        assert!(s.nodes.is_empty());
    }

    #[test]
    fn find_locates_node_by_ref() {
        let s = sample();
        assert_eq!(s.find("e1").unwrap().tag, "input");
        assert_eq!(s.find("e2").unwrap().tag, "button");
        assert!(s.find("e99").is_none());
    }
}
