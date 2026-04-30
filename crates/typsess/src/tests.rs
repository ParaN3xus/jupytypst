use super::*;

#[test]
fn top_level_text_set_persists_between_cells() {
    let mut session = svg_session();
    session.execute("set text(fill: red)\n[First]").unwrap();
    assert!(session_has_style_for(&session, "text", "fill"));
    assert!(svg_output(session.execute("[Second]").unwrap()).contains("<svg"));
}

#[test]
fn svg_mode_does_not_rerender_previous_visible_content() {
    let mut session = svg_session();
    session.execute("lorem(20)").unwrap();

    let output = session.execute("[Test]").unwrap();
    let svg = svg_output(output);
    assert!(svg.contains("<svg"));
    assert!(!svg.contains("Lorem"));
}

#[test]
fn code_context_persists_without_hash_prefix() {
    let mut session = svg_session();
    session.execute("let f(a, b) = a + b").unwrap();
    assert!(svg_output(session.execute("f(1, 2)").unwrap()).contains("<svg"));
}

#[test]
fn page_set_rules_do_not_persist_between_cells() {
    let mut session = svg_session();
    session.execute("set page(paper: \"a4\")\n[First]").unwrap();

    let svg = svg_output(session.execute("[Second]").unwrap());
    assert!(svg.contains("<svg"));
    assert!(!session_has_style_for(&session, "page", "paper"));
}

#[test]
fn page_setup_default_initializes_persistent_styles() {
    let session = svg_session();
    assert!(session_has_style_for(&session, "page", "width"));
    assert!(session_has_style_for(&session, "page", "height"));
    assert!(session_has_style_for(&session, "page", "margin"));
}

#[test]
fn default_page_setup_controls_rendered_svg_size() {
    let mut default_session = svg_session();
    let mut no_setup_session = TypstReplSession::new(RenderMode::Svg, PageSetup::None).unwrap();

    let default_svg = svg_output(default_session.execute("[x]").unwrap());
    let no_setup_svg = svg_output(no_setup_session.execute("[x]").unwrap());

    let default_width = svg_dimension(&default_svg, "width");
    let default_height = svg_dimension(&default_svg, "height");
    let no_setup_width = svg_dimension(&no_setup_svg, "width");
    let no_setup_height = svg_dimension(&no_setup_svg, "height");

    assert!(
        no_setup_width > default_width * 5.0,
        "default page setup did not shrink SVG width: default={default_width}, none={no_setup_width}"
    );
    assert!(
        no_setup_height > default_height * 5.0,
        "default page setup did not shrink SVG height: default={default_height}, none={no_setup_height}"
    );
}

#[test]
fn page_setup_none_does_not_initialize_page_styles() {
    let session = TypstReplSession::new(RenderMode::Svg, PageSetup::None).unwrap();
    assert!(!session_has_style_for(&session, "page", "width"));
    assert!(!session_has_style_for(&session, "page", "height"));
    assert!(!session_has_style_for(&session, "page", "margin"));
}

#[test]
fn page_setup_custom_initializes_persistent_styles() {
    let session = TypstReplSession::new(
        RenderMode::Svg,
        PageSetup::Custom("set page(fill: red)".into()),
    )
    .unwrap();
    assert!(session_has_style_for(&session, "page", "fill"));
}

#[test]
fn current_cell_page_size_overrides_default_but_does_not_persist() {
    let mut session = svg_session();
    let initial_width_count = session_style_count_for(&session, "page", "width");

    let wide_svg = svg_output(
        session
            .execute("set page(width: 300pt, height: 80pt)\n[x]")
            .unwrap(),
    );
    let next_svg = svg_output(session.execute("[x]").unwrap());

    assert!(svg_dimension(&wide_svg, "width") > 250.0);
    assert!(svg_dimension(&next_svg, "width") < 100.0);
    assert_eq!(
        session_style_count_for(&session, "page", "width"),
        initial_width_count
    );
}

#[test]
fn page_fill_persists_but_page_width_does_not() {
    let mut session = svg_session();
    let initial_width_count = session_style_count_for(&session, "page", "width");
    session
        .execute("set page(width: 3cm, fill: red)\n[First]")
        .unwrap();
    assert!(session_has_style_for(&session, "page", "fill"));
    assert_eq!(
        session_style_count_for(&session, "page", "width"),
        initial_width_count
    );
    assert!(svg_output(session.execute("[Second]").unwrap()).contains("<svg"));
}

#[test]
fn anonymous_show_rules_persist_between_cells() {
    let mut session = svg_session();
    session.execute("show: it => emph(it)\n[First]").unwrap();
    assert!(session.styles.iter().any(|style| style.recipe().is_some()));
    assert!(svg_output(session.execute("[Second]").unwrap()).contains("<svg"));
}

#[test]
fn selector_show_rules_persist_between_cells() {
    let mut session = svg_session();
    session
        .execute("show regex(\"x\"): set text(fill: red)\n[x]")
        .unwrap();
    assert!(session.styles.iter().any(|style| style.recipe().is_some()));
    assert!(svg_output(session.execute("[x]").unwrap()).contains("<svg"));
}

#[test]
fn state_updates_persist_between_cells_without_visible_content() {
    let mut session = html_session();
    let first = html_output(
        session
            .execute("let s = state(\"test\", \"init\")\ns.update(\"upd\")\ncontext s.get()")
            .unwrap(),
    );
    let second = html_output(session.execute("context s.get()").unwrap());

    assert!(first.contains("upd"));
    assert!(second.contains("<p>upd</p>"));
    assert!(!second.contains("<p>init</p>"));
    assert_eq!(second.matches("upd").count(), 1);
}

#[test]
fn svg_mode_returns_multiple_structured_pages() {
    let mut session = svg_session();
    let output = session.execute("[x]\n\npagebreak()\n\n[x]").unwrap();
    match output.output {
        ExecutionOutput::Paged(document) => assert!(document.pages.len() >= 2),
        other => panic!("unexpected output: {other:?}"),
    }
}

#[test]
fn execute_with_mode_renders_without_parsing_host_directives() {
    let mut session = html_session();
    let html = html_output(session.execute_with_mode("[x]", RenderMode::Html).unwrap());
    assert!(html.contains("<p>x</p>"));

    let svg = svg_output(session.execute_with_mode("[x]", RenderMode::Svg).unwrap());
    assert!(svg.contains("<svg"));
}

#[test]
fn classifies_complete_input() {
    assert_eq!(classify_input("let x = 1"), InputStatus::Complete);
}

#[test]
fn classifies_incomplete_input() {
    assert!(matches!(classify_input("("), InputStatus::Incomplete(_)));
    assert!(matches!(
        classify_input("\"abc"),
        InputStatus::Incomplete(_)
    ));
}

#[test]
fn classifies_invalid_input() {
    assert!(matches!(
        classify_input("let x = 1 2"),
        InputStatus::Invalid(_)
    ));
}

#[test]
fn world_inputs_are_visible_to_sys_inputs() {
    let mut session = TypstReplSession::new_with_world_options(
        RenderMode::Html,
        PageSetup::Default,
        WorldOptions {
            inputs: vec![("name".into(), "typst".into())],
            ..WorldOptions::default()
        },
    )
    .unwrap();

    let html = html_output(session.execute("sys.inputs.name").unwrap());
    assert!(html.contains("typst"));
}

#[test]
fn world_root_controls_relative_imports() {
    let temp_dir = tempfile::tempdir().unwrap();
    std::fs::write(temp_dir.path().join("defs.typ"), "#let value = [Imported]").unwrap();
    let mut session = TypstReplSession::new_with_world_options(
        RenderMode::Html,
        PageSetup::Default,
        WorldOptions {
            root: Some(temp_dir.path().to_path_buf()),
            ..WorldOptions::default()
        },
    )
    .unwrap();

    let html = html_output(
        session
            .execute("import \"defs.typ\": value\nvalue")
            .unwrap(),
    );
    assert!(html.contains("Imported"));
}

#[test]
fn code_block_errors_keep_inner_expression_span() {
    let mut session = html_session();
    let errors = session
        .execute("{\nstr(1 + 1)\npage.fill\n}")
        .expect_err("contextual page field access should fail outside context");
    let range = errors
        .first()
        .and_then(|diagnostic| diagnostic.span.range())
        .expect("diagnostic should have a source range");

    let source = "{\nstr(1 + 1)\npage.fill\n}";
    assert_eq!(&source[range.clone()], "fill");
    assert_eq!(source[..range.start].lines().count(), 3);
}

fn svg_session() -> TypstReplSession {
    TypstReplSession::new(RenderMode::Svg, PageSetup::Default).unwrap()
}

fn html_session() -> TypstReplSession {
    TypstReplSession::new(RenderMode::Html, PageSetup::Default).unwrap()
}

fn svg_output(result: ExecutionResult) -> String {
    match result.output {
        ExecutionOutput::Paged(document) => typst_svg::svg(&document.pages[0]),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn html_output(result: ExecutionResult) -> String {
    match result.output {
        ExecutionOutput::Html(document) => typst_html::html(&document).unwrap(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn session_has_style_for(session: &TypstReplSession, element: &str, field: &str) -> bool {
    session_style_count_for(session, element, field) > 0
}

fn session_style_count_for(session: &TypstReplSession, element: &str, field: &str) -> usize {
    session
        .styles
        .iter()
        .filter(|style| {
            let Some(property) = style.property() else {
                return false;
            };
            let Some(style_element) = style.element() else {
                return false;
            };
            style_element.name() == element
                && style_element
                    .field_id(field)
                    .is_some_and(|id| property.is(style_element, id))
        })
        .count()
}

fn svg_dimension(svg: &str, name: &str) -> f64 {
    let needle = format!(r#"{name}=""#);
    let start = svg.find(&needle).expect("missing SVG dimension") + needle.len();
    let rest = &svg[start..];
    let end = rest.find('"').expect("unterminated SVG dimension");
    rest[..end]
        .trim_end_matches("pt")
        .parse()
        .expect("invalid SVG dimension")
}
