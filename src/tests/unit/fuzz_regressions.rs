use crate::stack::description::Body;

#[test]
fn rendered_descriptions_canonicalise_new_line_endings_before_first_write() {
    for (existing, message, template) in [
        ("human\r", "", ""),
        ("\r\r", "", ""),
        ("human\r\r", "", ""),
        ("human\r\r\nbody", "", ""),
        ("", "commit\r\nbody", ""),
        ("", "commit\r\r\nbody", ""),
        ("", "", "template\r\nbody"),
        ("", "", "template\r\r\nbody"),
        ("", "commit\r", ""),
        ("", "", "template\r"),
    ] {
        let first = Body::parse(existing)
            .unwrap()
            .render_update(message, "Stack navigation", template)
            .unwrap()
            .unwrap();
        assert!(
            !first.contains("\r\n"),
            "{existing:?} / {message:?} / {template:?}"
        );
        assert_eq!(
            Body::parse(&first)
                .unwrap()
                .render_update(message, "Stack navigation", template)
                .unwrap(),
            None,
            "an unchanged footer must not require a canonicalisation-only write"
        );
    }
}

#[test]
fn rendering_preserves_bare_carriage_returns_without_line_feed() {
    for existing in ["human\rtext", "human\r\rtext", "human\r\rtext\nbody"] {
        let first = Body::parse(existing)
            .unwrap()
            .render_update("", "Stack navigation", "")
            .unwrap()
            .unwrap();
        assert!(first.starts_with(&format!("{existing}\n\n---\n")));
        assert_eq!(
            Body::parse(&first)
                .unwrap()
                .render_update("", "Stack navigation", "")
                .unwrap(),
            None
        );
    }
}
