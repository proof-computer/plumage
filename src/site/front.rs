//! Today's page.

use super::layout::{
    document, push_arguments, push_changes, push_current, push_fetches, push_live,
};
use super::{DayPage, Paint, Section};

/// Today's page, painted by `chrome.painted_by` (yesterday's result).
pub fn render_front(page: &DayPage<'_>) -> String {
    let mut body = String::new();
    body.push_str("<h1>Today ");
    body.push_str(&page.day.to_string());
    body.push_str("</h1>\n<p>");
    body.push_str(&yesterday(page.chrome.painted_by));
    body.push_str("</p>\n");
    push_current(&mut body, page);
    push_live(&mut body, page);
    push_changes(&mut body, page.votes, &page.chrome);
    push_fetches(&mut body, page.evidence, &page.chrome);
    push_arguments(&mut body, page.arguments, &page.chrome);
    document(
        &format!("Today {}", page.day),
        Some(Section::Today),
        0,
        &page.chrome,
        &body,
    )
}

fn yesterday(paint: Paint) -> String {
    match paint {
        Paint::Decided(colour) => format!("Painted by yesterday: {}.", colour.name()),
        Paint::Undecided => "Yesterday was undecided under the rule".to_owned(),
        Paint::NoRecord => "No result is recorded for yesterday".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::render_front;
    use crate::colour::Colour::{Black, Yellow};
    use crate::site::support::{assert_clean, body_open, page, page_named};
    use crate::site::{ArgumentView, Paint};

    #[test]
    fn render_front_paints_black_with_white_text_and_yellow_with_black_text() {
        let black = page(&[], &[], &[], None, Paint::Decided(Black));
        let html = render_front(&black);
        assert!(html.contains("<body style=\"background:#000000;color:#FFFFFF\">"));
        assert!(html.contains("background:#000000"));
        assert!(html.contains("color:#FFFFFF"));
        assert_clean(&html);

        let yellow = page(&[], &[], &[], None, Paint::Decided(Yellow));
        let html = render_front(&yellow);
        assert!(html.contains("<body style=\"background:#FFFF00;color:#000000\">"));
        assert!(html.contains("background:#FFFF00"));
        assert!(html.contains("color:#000000"));
        assert!(!html.contains("color:#FFFFFF"));
    }

    #[test]
    fn render_front_undecided_and_no_record_paint_nothing() {
        let undecided = page(&[], &[], &[], None, Paint::Undecided);
        let html = render_front(&undecided);
        assert!(html.contains("Yesterday was undecided under the rule"));
        assert_eq!(body_open(&html), "<body>");
        assert_clean(&html);

        let missing = page(&[], &[], &[], None, Paint::NoRecord);
        let html = render_front(&missing);
        assert!(html.contains("No result is recorded for yesterday"));
        assert_eq!(body_open(&html), "<body>");
    }

    #[test]
    fn named_peer_is_shown_beside_the_id_and_escaped() {
        let mut names = BTreeMap::new();
        names.insert("peer-a".to_owned(), "<Wren&>".to_owned());
        let arguments = [
            ArgumentView {
                id: "arg-1".to_owned(),
                author: "peer-a".to_owned(),
                at_ms: 0,
                text: "named".to_owned(),
                cites: Vec::new(),
            },
            ArgumentView {
                id: "arg-2".to_owned(),
                author: "ada".to_owned(),
                at_ms: 1,
                text: "unnamed".to_owned(),
                cites: Vec::new(),
            },
        ];
        let shown = page_named(
            &[],
            &arguments,
            &[],
            None,
            Paint::NoRecord,
            "peer-a",
            &names,
        );
        let html = render_front(&shown);
        assert!(html.contains("&lt;Wren&amp;&gt; <code>peer-a</code>"));
        assert!(!html.contains("<Wren&>"));
        assert!(html.contains("<p><code>ada</code></p>"));
        assert!(html.contains(
            "Records as of 00:00:00 UTC on 1970-01-01; rendered 1970-01-01 01:02:03 UTC by &lt;Wren&amp;&gt; (peer-a)"
        ));
        assert_clean(&html);
    }
}
