// SPDX-License-Identifier: GPL-3.0-or-later

use flowmux_md_viewer::render_markdown_body;

struct GfmExample {
    number: usize,
    markdown: String,
    html: String,
}

#[test]
fn renders_gfm_spec_examples() {
    let examples = gfm_examples(include_str!("fixtures/gfm-spec.txt"));
    assert_eq!(examples.len(), 672);

    for example in examples {
        let html = render_markdown_body(&example.markdown);
        assert!(
            example.html.is_empty() || !html.is_empty(),
            "GFM example {} rendered empty HTML for non-empty expected HTML",
            example.number
        );
        if is_core_gfm_extension_example(example.number) {
            assert_eq!(
                html, example.html,
                "GFM example {} did not match expected HTML",
                example.number
            );
        }
    }
}

fn is_core_gfm_extension_example(number: usize) -> bool {
    matches!(number, 198..=205 | 491..=502 | 536..=544 | 631..=635)
}

fn gfm_examples(spec: &str) -> Vec<GfmExample> {
    let mut examples = Vec::new();
    let mut lines = spec.lines();

    while let Some(line) = lines.next() {
        let mut header = line.split_whitespace();
        let Some(fence) = header.next() else {
            continue;
        };
        if !fence.chars().all(|ch| ch == '`') || header.next() != Some("example") {
            continue;
        }

        let number = examples.len() + 1;
        let mut markdown = String::new();
        let mut html = String::new();
        let mut in_html = false;
        for example_line in lines.by_ref() {
            if example_line == fence {
                break;
            }
            if example_line == "." {
                in_html = true;
                continue;
            }
            if in_html {
                html.push_str(&example_line.replace('→', "\t"));
                html.push('\n');
            } else {
                markdown.push_str(&example_line.replace('→', "\t"));
                markdown.push('\n');
            }
        }

        examples.push(GfmExample {
            number,
            markdown,
            html,
        });
    }

    examples
}
