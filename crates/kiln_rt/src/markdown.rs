//! The `markdown` module: CommonMark, with GitHub's tables, strikethrough
//! and task lists, on the pulldown-cmark crate.

use std::collections::HashMap;

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};

use crate::html::{is_safe_url, UNSAFE_URL};
use crate::value::{bytes, kv, ptr, string, KStr};

/// The HTML for the Markdown in `s`.
///
/// # Safety
///
/// `s` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_markdown_to_html(s: *mut KStr) -> *mut KStr {
    let text = String::from_utf8_lossy(bytes(kv(s)));
    ptr(string(to_html(&text).as_bytes()))
}

/// HTML written in the Markdown is shown as text, not kept, and links and
/// images go only to URLs that `html.safe_url` keeps, so the result is as
/// safe to place in a page as escaped text. Headings get ids from
/// their text, as GitHub gives them, so `#a-heading` links to them.
fn to_html(text: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut events: Vec<Event> = Parser::new_ext(text, options)
        .map(|e| match e {
            Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: safe(dest_url),
                title,
                id,
            }),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Image {
                link_type,
                dest_url: safe(dest_url),
                title,
                id,
            }),
            other => other,
        })
        .collect();
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { id: None, .. }) = &events[i] {
            let mut words = String::new();
            for e in &events[i + 1..] {
                match e {
                    Event::End(TagEnd::Heading(_)) => break,
                    Event::Text(t) | Event::Code(t) => words.push_str(t),
                    _ => {}
                }
            }
            let mut slug = slug(&words);
            let n = used.entry(slug.clone()).or_insert(0);
            if *n > 0 {
                slug = format!("{slug}-{n}");
            }
            *n += 1;
            if let Event::Start(Tag::Heading { id, .. }) = &mut events[i] {
                *id = Some(CowStr::from(slug));
            }
        }
        i += 1;
    }
    let mut out = String::with_capacity(text.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    out
}

fn safe(url: CowStr) -> CowStr {
    if is_safe_url(url.as_bytes()) {
        url
    } else {
        CowStr::from(UNSAFE_URL)
    }
}

/// A heading's id: its words in lower case, joined by `-`.
fn slug(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_commonmark_with_tables_and_heading_ids() {
        let html = to_html("# The `ui` key\n\nSome *text*.\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n## The `ui` key\n");
        assert!(html.starts_with("<h1 id=\"the-ui-key\">The <code>ui</code> key</h1>\n<p>Some <em>text</em>.</p>\n<table>"), "{html}");
        assert!(
            html.ends_with("<h2 id=\"the-ui-key-1\">The <code>ui</code> key</h2>\n"),
            "{html}"
        );
    }

    #[test]
    fn html_in_markdown_is_text() {
        assert_eq!(
            to_html("<script>alert(1)</script>\n\nA <b>bold</b> [link](javascript:x).\n"),
            "&lt;script&gt;alert(1)&lt;/script&gt;\n<p>A &lt;b&gt;bold&lt;/b&gt; <a href=\"about:invalid#kiln-unsafe-url\">link</a>.</p>\n"
        );
    }
}
