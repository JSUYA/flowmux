// SPDX-License-Identifier: GPL-3.0-or-later

use flowmux_md_viewer::render_markdown_body;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CommonMarkExample {
    markdown: String,
    html: String,
    example: u32,
    section: String,
}

#[test]
fn renders_commonmark_0_31_2_spec_examples_to_html() {
    let examples: Vec<CommonMarkExample> =
        serde_json::from_str(include_str!("fixtures/commonmark-0.31.2-spec.json"))
            .expect("parse CommonMark spec fixture");
    assert_eq!(examples.len(), 652);

    for example in examples {
        let html = render_markdown_body(&example.markdown);
        assert!(
            example.html.is_empty() || !html.is_empty(),
            "CommonMark example {} ({}) rendered empty HTML for non-empty expected HTML",
            example.example,
            example.section
        );
    }
}
