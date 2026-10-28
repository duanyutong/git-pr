use super::{clip, complete_lines};
use unicode_width::UnicodeWidthStr as _;

#[test]
fn ellipsis_preserves_graphemes_and_uses_only_available_cells() {
    for text in [
        "",
        "café",
        "e\u{301}clair",
        "日本語",
        "👩‍💻abc",
        "\u{f407} PR",
    ] {
        for columns in 0..=30 {
            let shown = clip(text, columns);
            assert!(shown.width() <= columns);
            if text.width() <= columns {
                assert_eq!(shown, text);
            } else if columns != 0 {
                assert!(shown.ends_with('…'));
            }
        }
    }
    assert_eq!(clip("日本語", 3), "日…");
    assert_eq!(clip("e\u{301}clair", 2), "e\u{301}…");
}

#[test]
fn facts_reflow_without_losing_numeric_values_or_long_identities() {
    let text = "GraphQL 0/5000 · retry 12s · org/long-repository";
    for columns in 0..=80 {
        let lines = complete_lines(text, columns);
        assert_eq!(lines.join(" "), text);
        assert!(lines.iter().any(|line| line.contains("0/5000")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("org/long-repository"))
        );
    }
    assert_eq!(complete_lines("", 80), [""]);
}
