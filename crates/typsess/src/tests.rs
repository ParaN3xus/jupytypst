use super::*;
use std::ops::ControlFlow;
use std::sync::Arc;

use ecow::eco_vec;
use typst::foundations::{Element, Selector};
use typst::introspection::{Counter, State};
use typst::layout::PageElem;

const DEFAULT_TEST_PAGE_SETUP: &str = "set page(width: auto, height: auto, margin: 16pt)";

#[test]
fn top_level_text_set_persists_between_cells() {
    let mut session = code_svg_session();
    session.execute("set text(fill: red)\n[First]").unwrap();
    assert!(session_has_style_for(&session, "text", "fill"));
    assert!(svg_output(session.execute("[Second]").unwrap()).contains("<svg"));
}

#[test]
fn svg_mode_does_not_rerender_previous_visible_content() {
    let mut session = code_svg_session();
    session.execute("lorem(20)").unwrap();

    let output = session.execute("[Test]").unwrap();
    let svg = svg_output(output);
    assert!(svg.contains("<svg"));
    assert!(!svg.contains("Lorem"));
}

#[test]
fn code_context_persists_without_hash_prefix() {
    let mut session = code_svg_session();
    session.execute("let f(a, b) = a + b").unwrap();
    assert!(svg_output(session.execute("f(1, 2)").unwrap()).contains("<svg"));
}

#[test]
fn page_set_rules_do_not_persist_between_cells() {
    let mut session = code_svg_session();
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
    let mut no_setup_session = test_session(
        RenderMode::Svg,
        SourceMode::Markup,
        "",
        WorldOptions::default(),
    );

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
    let session = test_session(
        RenderMode::Svg,
        SourceMode::Markup,
        "",
        WorldOptions::default(),
    );
    assert!(!session_has_style_for(&session, "page", "width"));
    assert!(!session_has_style_for(&session, "page", "height"));
    assert!(!session_has_style_for(&session, "page", "margin"));
}

#[test]
fn page_setup_custom_initializes_persistent_styles() {
    let session = test_session(
        RenderMode::Svg,
        SourceMode::Markup,
        "set page(fill: red)",
        WorldOptions::default(),
    );
    assert!(session_has_style_for(&session, "page", "fill"));
}

#[test]
fn current_cell_page_size_overrides_default_but_does_not_persist() {
    let mut session = code_svg_session();
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
    let mut session = code_svg_session();
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
    let mut session = code_svg_session();
    session.execute("show: it => emph(it)\n[First]").unwrap();
    assert!(session.styles.iter().any(|style| style.recipe().is_some()));
    assert!(svg_output(session.execute("[Second]").unwrap()).contains("<svg"));
}

#[test]
fn selector_show_rules_persist_between_cells() {
    let mut session = code_svg_session();
    session
        .execute("show regex(\"x\"): set text(fill: red)\n[x]")
        .unwrap();
    assert!(session.styles.iter().any(|style| style.recipe().is_some()));
    assert!(svg_output(session.execute("[x]").unwrap()).contains("<svg"));
}

#[test]
fn state_updates_persist_between_cells_without_visible_content() {
    let mut session = code_html_session();
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
    let mut session = code_svg_session();
    let output = session.execute("[x]\n\npagebreak()\n\n[x]").unwrap();
    match output.output {
        ExecutionOutput::Paged(document) => assert!(document.pages.len() >= 2),
        other => panic!("unexpected output: {other:?}"),
    }
}

#[test]
fn execute_with_mode_renders_without_parsing_host_directives() {
    let mut session = code_html_session();
    let html = html_output(session.execute_with_mode("[x]", RenderMode::Html).unwrap());
    assert!(html.contains("<p>x</p>"));

    let svg = svg_output(session.execute_with_mode("[x]", RenderMode::Svg).unwrap());
    assert!(svg.contains("<svg"));
}

#[test]
fn classifies_complete_input() {
    assert_eq!(
        classify_input("let x = 1", SourceMode::Code),
        InputStatus::Complete
    );
}

#[test]
fn classifies_incomplete_input() {
    assert!(matches!(
        classify_input("(", SourceMode::Code),
        InputStatus::Incomplete(_)
    ));
    assert!(matches!(
        classify_input("\"abc", SourceMode::Code),
        InputStatus::Incomplete(_)
    ));
}

#[test]
fn classifies_invalid_input() {
    assert!(matches!(
        classify_input("let x = 1 2", SourceMode::Code),
        InputStatus::Invalid(_)
    ));
}

#[test]
fn markup_mode_executes_markup_source() {
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    );
    let html = html_output(session.execute("Hello\n#let x = 1\n#x").unwrap());
    assert!(html.contains("Hello"));
    assert!(html.contains("1"));
}

#[test]
fn markup_mode_persists_definitions_with_hash_prefix() {
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    );
    session.execute("#let f(a, b) = a + b").unwrap();
    let html = html_output(session.execute("#f(1, 2)").unwrap());
    assert!(html.contains("3"));
}

#[test]
fn persisted_markup_functions_keep_previous_source_spans() {
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    );
    session.execute("#let f() = panic()").unwrap();
    session.execute("#let g() = f()").unwrap();
    let errors = session.execute("#g()").unwrap_err();
    let diagnostic = errors.first().expect("expected a Typst diagnostic");
    let sources = session.diagnostic_sources();
    let first_user_source = sources
        .iter()
        .find(|source| source.source == "#let f() = panic()")
        .expect("missing first input source");
    let current_user_source = sources
        .iter()
        .find(|source| source.source == "#g()")
        .expect("missing current input source");
    let second_user_source = sources
        .iter()
        .find(|source| source.source == "#let g() = f()")
        .expect("missing second input source");

    assert_eq!(diagnostic.span.id(), Some(first_user_source.id));
    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.span.id() == Some(second_user_source.id)),
        "missing second input trace: {diagnostic:?}"
    );
    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.span.id() == Some(current_user_source.id))
    );
}

#[test]
fn code_only_markup_cell_preserves_call_trace() {
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    );
    let errors = session
        .execute("#let f() = panic()\n\n#let g() = f()\n\n\n#g()")
        .unwrap_err();
    let diagnostic = errors.first().expect("expected a Typst diagnostic");

    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.v.to_string().contains("function `f`")),
        "missing f call trace: {diagnostic:?}"
    );
    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.v.to_string().contains("function `g`")),
        "missing g call trace: {diagnostic:?}"
    );
}

#[test]
fn new_session_defaults_to_markup_mode() {
    let mut session = TypstReplSession::new(SessionOptions::default()).unwrap();
    let html = html_output(session.execute("Hello\n#let x = 1\n#x").unwrap());
    assert!(html.contains("Hello"));
    assert!(html.contains("1"));
}

#[test]
fn world_inputs_are_visible_to_sys_inputs() {
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Code,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions {
            inputs: vec![("name".into(), "typst".into())],
            ..WorldOptions::default()
        },
    );

    let html = html_output(session.execute("sys.inputs.name").unwrap());
    assert!(html.contains("typst"));
}

#[test]
fn world_root_controls_relative_imports() {
    let temp_dir = tempfile::tempdir().unwrap();
    std::fs::write(temp_dir.path().join("defs.typ"), "#let value = [Imported]").unwrap();
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Code,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions {
            root: Some(temp_dir.path().to_path_buf()),
            ..WorldOptions::default()
        },
    );

    let html = html_output(
        session
            .execute("import \"defs.typ\": value\nvalue")
            .unwrap(),
    );
    assert!(html.contains("Imported"));
}

#[test]
fn diagnostics_include_imported_source_tracepoints() {
    let temp_dir = tempfile::tempdir().unwrap();
    std::fs::write(temp_dir.path().join("defs.typ"), "#let f() = panic()").unwrap();
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Code,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions {
            root: Some(temp_dir.path().to_path_buf()),
            ..WorldOptions::default()
        },
    );

    session
        .execute("import \"defs.typ\": f\nf()")
        .expect_err("imported function should panic");
    assert!(
        session
            .diagnostic_sources()
            .iter()
            .any(|source| source.name.ends_with("/defs.typ")),
        "missing imported diagnostic source"
    );
}

#[test]
fn single_importing_cell_keeps_user_and_imported_tracepoints() {
    let temp_dir = tempfile::tempdir().unwrap();
    std::fs::write(temp_dir.path().join("defs.typ"), "#let invoke(f) = f()").unwrap();
    let mut session = test_session(
        RenderMode::Html,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions {
            root: Some(temp_dir.path().to_path_buf()),
            ..WorldOptions::default()
        },
    );

    let errors = session
        .execute(
            "#import \"defs.typ\": invoke\n\
             #let f = () => panic()\n\n\
             #invoke(f)",
        )
        .expect_err("callback should panic");
    let sources = session.diagnostic_sources();
    let user_source = sources
        .iter()
        .find(|source| source.source.contains("panic()"))
        .expect("missing user diagnostic source");
    let imported_source = sources
        .iter()
        .find(|source| source.name.ends_with("/defs.typ"))
        .expect("missing imported diagnostic source");
    let diagnostic = errors.first().expect("expected a diagnostic");

    assert_eq!(diagnostic.span.id(), Some(user_source.id));
    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.span.id() == Some(imported_source.id)),
        "missing imported tracepoint: {diagnostic:?}"
    );
    assert!(
        diagnostic
            .trace
            .iter()
            .any(|trace| trace.span.id() == Some(user_source.id)),
        "missing user tracepoint: {diagnostic:?}"
    );
}

#[test]
fn code_block_errors_keep_inner_expression_span() {
    let mut session = code_html_session();
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
    test_session(
        RenderMode::Svg,
        SourceMode::Markup,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    )
}

fn code_svg_session() -> TypstReplSession {
    test_session(
        RenderMode::Svg,
        SourceMode::Code,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    )
}

fn code_html_session() -> TypstReplSession {
    test_session(
        RenderMode::Html,
        SourceMode::Code,
        DEFAULT_TEST_PAGE_SETUP,
        WorldOptions::default(),
    )
}

fn test_session(
    render_mode: RenderMode,
    source_mode: SourceMode,
    page_setup: &'static str,
    world_options: WorldOptions,
) -> TypstReplSession {
    let state = initial_test_state(page_setup, world_options.clone());
    TypstReplSession::new(SessionOptions {
        render_mode,
        source_mode,
        world_options,
        state,
        persistence: test_persistence(),
    })
    .unwrap()
}

fn initial_test_state(page_setup: &'static str, world_options: WorldOptions) -> SessionState {
    let mut session = TypstReplSession::new(SessionOptions {
        world_options,
        ..SessionOptions::default()
    })
    .unwrap();
    if !page_setup.is_empty() {
        session.apply_source(page_setup, SourceMode::Code).unwrap();
    }
    session.into_state()
}

fn test_persistence() -> SessionPersistence {
    SessionPersistence {
        filter_styles: Arc::new(filter_test_persistent_styles),
        collect_introspection_updates: Arc::new(collect_test_introspection_updates),
    }
}

fn filter_test_persistent_styles(styles: Styles) -> Styles {
    styles
        .into_iter()
        .filter(|style| {
            style
                .property()
                .is_none_or(|property| !is_test_transient_page_property(property))
        })
        .collect()
}

fn is_test_transient_page_property(property: &typst::foundations::Property) -> bool {
    let page = Element::of::<PageElem>();
    ["paper", "width", "height"]
        .into_iter()
        .filter_map(|field| page.field_id(field))
        .any(|id| property.is(page, id))
}

fn collect_test_introspection_updates(content: &Content) -> Vec<Content> {
    let selector = Selector::Or(eco_vec![State::select_any(), Counter::select_any()]);
    let mut updates = Vec::new();
    let _ = content.traverse(&mut |element| {
        if selector.matches(&element, None) {
            updates.push(element);
        }
        ControlFlow::<()>::Continue(())
    });
    updates
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
