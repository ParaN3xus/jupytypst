use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use comemo::{Constraint, Track};
use ecow::{EcoVec, eco_vec};
use tinymist_world::args::CompileFontArgs;
use tinymist_world::system::{SystemUniverseBuilder, TypstSystemWorld};
use tinymist_world::{EntryState, ShadowApi};
use typst::World;
use typst::diag::{At, SourceDiagnostic};
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{
    Bytes, Content, Context, Element, Scope, Scopes, Selector, Style, StyleChain, Styles, Target,
    TargetElem, Value, ops,
};
use typst::introspection::{Counter, Introspector, State};
use typst::layout::{PageElem, PagedDocument};
use typst::syntax::{Span, VirtualPath, ast, ast::AstNode, parse_code};
use typst_eval::{Eval, Vm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Svg,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSetup {
    Default,
    None,
    Custom(String),
}

impl PageSetup {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "default" => Ok(Self::Default),
            "none" => Ok(Self::None),
            "" => Err(anyhow!("page setup cannot be empty")),
            custom => Ok(Self::Custom(custom.to_string())),
        }
    }

    fn code(&self) -> Option<&str> {
        match self {
            Self::Default => Some("set page(width: auto, height: auto, margin: 16pt)"),
            Self::None => None,
            Self::Custom(code) => Some(code.as_str()),
        }
    }
}

#[derive(Debug)]
pub enum ExecutionOutput {
    Svg(String),
    Html(String),
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub output: ExecutionOutput,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputStatus {
    Complete,
    Incomplete(String),
    Invalid(String),
}

pub fn classify_input(source: &str) -> InputStatus {
    let errors = parse_code(source).errors();
    if errors.is_empty() {
        return InputStatus::Complete;
    }

    let message = errors
        .into_iter()
        .map(|error| error.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    if is_incomplete_input(&message) {
        InputStatus::Incomplete(message)
    } else {
        InputStatus::Invalid(message)
    }
}

pub struct TypstReplSession {
    render_mode: RenderMode,
    scope: Scope,
    styles: Styles,
    introspection_updates: Vec<Content>,
    world: TypstSystemWorld,
}

impl TypstReplSession {
    pub fn new(render_mode: RenderMode, page_setup: PageSetup) -> Result<Self> {
        let world = create_world()?;
        let mut session = Self {
            render_mode,
            scope: Scope::new(),
            styles: Styles::new(),
            introspection_updates: Vec::new(),
            world,
        };
        session.initialize_page_setup(page_setup)?;
        Ok(session)
    }

    pub fn execute(&mut self, source: &str) -> Result<ExecutionResult> {
        self.execute_with_mode(source, self.render_mode)
    }

    pub fn execute_with_mode(
        &mut self,
        source: &str,
        render_mode: RenderMode,
    ) -> Result<ExecutionResult> {
        let evaluated = self.evaluate_code(source)?;
        let content = self.with_introspection_context(evaluated.content.clone());
        let output = match render_mode {
            RenderMode::Svg => self.render_svg(content)?,
            RenderMode::Html => self.render_html(content)?,
        };
        self.introspection_updates
            .extend(collect_introspection_updates(&evaluated.content));
        Ok(ExecutionResult {
            output,
            warnings: evaluated.warnings,
        })
    }

    fn initialize_page_setup(&mut self, page_setup: PageSetup) -> Result<()> {
        if let Some(page_setup) = page_setup.code() {
            let setup = normalize_code_statement(page_setup);
            let evaluated = self.evaluate_source(setup, StyleCapture::Local)?;
            self.styles.apply(evaluated.captured_styles);
        }
        Ok(())
    }

    fn evaluate_code(&mut self, code: &str) -> Result<EvaluatedCell> {
        let evaluated = self.evaluate_source(code, StyleCapture::Persistent)?;
        self.scope = evaluated.scope;
        self.styles.apply(evaluated.captured_styles);

        let content = evaluated
            .value
            .display()
            .styled_with_map(self.styles.clone());

        Ok(EvaluatedCell {
            content,
            warnings: evaluated.warnings,
        })
    }

    fn evaluate_source(
        &mut self,
        source: &str,
        style_capture: StyleCapture,
    ) -> Result<EvaluatedSource> {
        self.world
            .map_shadow_by_id(self.world.main(), Bytes::from_string(source.to_string()))
            .map_err(|error| anyhow!("failed to update Typst main source: {error}"))?;

        let span = Span::from_range(self.world.main(), 0..source.len());
        let mut root = parse_code(source);
        root.synthesize(span);

        let errors = root.errors();
        if !errors.is_empty() {
            return Err(format_diagnostics(
                errors.into_iter().map(Into::into).collect(),
            ));
        }

        let mut sink = Sink::new();
        let mut captured_styles = Styles::new();
        let mut warnings = Vec::new();
        let world = self.world.html_task();
        let (value, new_scope, sink_warnings) = {
            let introspector = Introspector::default();
            let traced = Traced::default();
            let engine = Engine {
                routines: &typst::ROUTINES,
                world: (world.as_ref() as &dyn World).track(),
                introspector: introspector.track(),
                traced: traced.track(),
                sink: sink.track_mut(),
                route: Route::default(),
            };
            let context = Context::none();
            let mut scopes = Scopes::new(Some(world.library()));
            scopes.top = self.scope.clone();
            let mut vm = Vm::new(engine, context.track(), scopes, root.span());
            let value = eval_code_capture(
                &mut vm,
                &mut root.cast::<ast::Code>().unwrap().exprs(),
                style_capture,
                &mut captured_styles,
                &mut warnings,
            )
            .map_err(format_diagnostics)?;
            if let Some(flow) = vm.flow {
                return Err(format_diagnostics(eco_vec![flow.forbidden()]));
            }
            let new_scope = vm.scopes.top.clone();
            drop(vm);
            (value, new_scope, sink.warnings())
        };

        warnings.extend(format_warnings(sink_warnings));

        Ok(EvaluatedSource {
            value,
            scope: new_scope,
            captured_styles,
            warnings,
        })
    }

    fn render_svg(&self, content: Content) -> Result<ExecutionOutput> {
        let world = self.world.paged_task();
        let document =
            layout_paged_document(world.as_ref(), &content).map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Svg(svg_pages_html(&document)))
    }

    fn render_html(&self, content: Content) -> Result<ExecutionOutput> {
        let world = self.world.html_task();
        let document =
            layout_html_document(world.as_ref(), &content).map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Html(
            typst_html::html(&document).map_err(format_diagnostics)?,
        ))
    }

    fn with_introspection_context(&self, content: Content) -> Content {
        if self.introspection_updates.is_empty() {
            return content;
        }

        Content::sequence(
            self.introspection_updates
                .iter()
                .cloned()
                .chain(std::iter::once(content)),
        )
    }
}

impl Default for TypstReplSession {
    fn default() -> Self {
        Self::new(RenderMode::Html, PageSetup::Default).expect("default page setup should be valid")
    }
}

fn normalize_code_statement(code: &str) -> &str {
    code.trim_start_matches('#').trim_start()
}

fn is_incomplete_input(message: &str) -> bool {
    let message = message.trim();
    message.starts_with("unclosed ")
        || [
            "expected expression",
            "expected block",
            "expected argument list",
            "expected identifier",
            "expected pattern",
            "expected colon",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix))
}

struct EvaluatedCell {
    content: Content,
    warnings: Vec<String>,
}

struct EvaluatedSource {
    value: Value,
    scope: Scope,
    captured_styles: Styles,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StyleCapture {
    Local,
    Persistent,
}

fn eval_code_capture<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
    style_capture: StyleCapture,
    captured_styles: &mut Styles,
    warnings: &mut Vec<String>,
) -> typst::diag::SourceResult<Value> {
    let flow = vm.flow.take();
    let mut output = Value::None;

    while let Some(expr) = exprs.next() {
        let span = expr.span();
        let value = match expr {
            ast::Expr::SetRule(set) => {
                let styles = set.eval(vm)?;
                match style_capture {
                    StyleCapture::Local => captured_styles.apply(styles.clone()),
                    StyleCapture::Persistent => {
                        captured_styles.apply(filter_persistent_styles(styles.clone()));
                    }
                }
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, style_capture, captured_styles, warnings)?
                    .display();
                Value::Content(tail.styled_with_map(styles))
            }
            ast::Expr::ShowRule(show) => {
                let recipe = show.eval(vm)?;
                let is_anonymous = recipe.selector().is_none();
                match style_capture {
                    StyleCapture::Local => {
                        captured_styles.apply(Style::from(recipe.clone()).into());
                    }
                    StyleCapture::Persistent => {
                        if is_anonymous {
                            warnings.push(
                            "jupytypst: anonymous `show: ...` rules are cell-local and are not persisted"
                                .to_string(),
                        );
                        } else {
                            captured_styles.apply(Style::from(recipe.clone()).into());
                        }
                    }
                }
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, style_capture, captured_styles, warnings)?
                    .display();
                Value::Content(tail.styled_with_recipe(&mut vm.engine, vm.context, recipe)?)
            }
            ast::Expr::CodeBlock(block) => block.eval(vm)?,
            _ => expr.eval(vm)?,
        };

        output = ops::join(output, value).at(span)?;

        if vm.flow.is_some() {
            break;
        }
    }

    if flow.is_some() {
        vm.flow = flow;
    }

    Ok(output)
}

fn filter_persistent_styles(styles: Styles) -> Styles {
    styles
        .into_iter()
        .filter(|style| {
            style
                .property()
                .is_none_or(|property| !is_transient_page_property(property))
        })
        .collect()
}

fn is_transient_page_property(property: &typst::foundations::Property) -> bool {
    let page = Element::of::<PageElem>();
    ["paper", "width", "height"]
        .into_iter()
        .filter_map(|field| page.field_id(field))
        .any(|id| property.is(page, id))
}

fn collect_introspection_updates(content: &Content) -> Vec<Content> {
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

fn format_diagnostics(diagnostics: EcoVec<SourceDiagnostic>) -> anyhow::Error {
    let message = diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    anyhow!(message)
}

fn format_warnings(warnings: EcoVec<SourceDiagnostic>) -> Vec<String> {
    warnings
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect()
}

fn layout_paged_document(
    world: &dyn World,
    content: &Content,
) -> typst::diag::SourceResult<PagedDocument> {
    let library = world.library();
    let base = StyleChain::new(&library.styles);
    let target_style = TargetElem::target.set(Target::Paged).wrap();
    let styles = base.chain(&target_style);
    let empty_introspector = Introspector::default();
    let traced = Traced::default();
    let mut previous = None;

    for iteration in 0..5 {
        let current_introspector = previous
            .as_ref()
            .map(|document: &PagedDocument| &document.introspector)
            .unwrap_or(&empty_introspector);
        let constraint = Constraint::new();
        let mut sink = Sink::new();
        let document = {
            let mut engine = Engine {
                routines: &typst::ROUTINES,
                world: world.track(),
                introspector: current_introspector.track_with(&constraint),
                traced: traced.track(),
                sink: sink.track_mut(),
                route: Route::default(),
            };
            typst_layout::layout_document(&mut engine, content, styles)?
        };

        let delayed = sink.delayed();
        if !delayed.is_empty() {
            return Err(delayed);
        }

        if constraint.validate(&document.introspector) || iteration == 4 {
            return Ok(document);
        }

        previous = Some(document);
    }

    unreachable!("layout loop always returns within five iterations")
}

fn layout_html_document(
    world: &dyn World,
    content: &Content,
) -> typst::diag::SourceResult<typst_html::HtmlDocument> {
    let library = world.library();
    let base = StyleChain::new(&library.styles);
    let target_style = TargetElem::target.set(Target::Html).wrap();
    let styles = base.chain(&target_style);
    let introspector = Introspector::default();
    let traced = Traced::default();
    let mut previous = None;

    for iteration in 0..5 {
        let current_introspector = previous
            .as_ref()
            .map(|document: &typst_html::HtmlDocument| &document.introspector)
            .unwrap_or(&introspector);
        let constraint = Constraint::new();
        let mut sink = Sink::new();
        let document = {
            let mut engine = Engine {
                routines: &typst::ROUTINES,
                world: world.track(),
                introspector: current_introspector.track_with(&constraint),
                traced: traced.track(),
                sink: sink.track_mut(),
                route: Route::default(),
            };
            typst_html::html_document(&mut engine, content, styles)?
        };

        let delayed = sink.delayed();
        if !delayed.is_empty() {
            return Err(delayed);
        }

        if constraint.validate(&document.introspector) || iteration == 4 {
            return Ok(document);
        }

        previous = Some(document);
    }

    unreachable!("layout loop always returns within five iterations")
}

fn svg_pages_html(document: &PagedDocument) -> String {
    let pages = document
        .pages
        .iter()
        .map(|page| {
            format!(
                r#"<div class="jupytypst-page">{}</div>"#,
                typst_svg::svg(page)
            )
        })
        .collect::<String>();
    format!(
        r#"<style>
.jupytypst-pages {{
  display: flex;
  flex-direction: column;
  gap: 12px;
  align-items: flex-start;
}}
.jupytypst-page {{
  max-width: 100%;
  overflow: auto;
}}
.jupytypst-page > svg {{
  display: block;
  max-width: 100%;
  height: auto;
}}
</style>
<div class="jupytypst-pages">{pages}</div>"#
    )
}

fn create_world() -> Result<TypstSystemWorld> {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry = EntryState::new_rooted(
        root.into(),
        Some(VirtualPath::new(Path::new("/__jupytypst__.typ"))),
    );
    let fonts = SystemUniverseBuilder::resolve_fonts(CompileFontArgs::default())?;
    let package_registry = SystemUniverseBuilder::resolve_package(None, None);
    let universe =
        SystemUniverseBuilder::build(entry, Default::default(), fonts.into(), package_registry);
    Ok(universe.snapshot())
}

#[cfg(test)]
mod tests {
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
        match output.output {
            ExecutionOutput::Svg(svg) => {
                assert!(svg.contains("<svg"));
                assert!(!svg.contains("Lorem"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn code_context_persists_without_hash_prefix() {
        let mut session = svg_session();
        session.execute("let f(a, b) = a + b").unwrap();
        let output = session.execute("f(1, 2)").unwrap();
        match output.output {
            ExecutionOutput::Svg(html) => assert!(html.contains("<svg")),
            other => panic!("unexpected output: {other:?}"),
        }
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
            PageSetup::Custom("#set page(fill: red)".into()),
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
    fn anonymous_show_rules_warn_and_do_not_persist() {
        let mut session = svg_session();
        let result = session.execute("show: it => emph(it)\n[First]").unwrap();
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("anonymous `show: ...`"))
        );
        let svg = svg_output(session.execute("[Second]").unwrap());
        assert!(svg.contains("<svg"));
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
    fn svg_mode_wraps_multiple_pages_as_independent_svgs() {
        let mut session = svg_session();
        let output = session.execute("[x]\n\npagebreak()\n\n[x]").unwrap();
        match output.output {
            ExecutionOutput::Svg(html) => {
                assert!(html.contains("jupytypst-pages"));
                assert!(html.matches("<svg").count() >= 2);
            }
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

    fn svg_session() -> TypstReplSession {
        TypstReplSession::new(RenderMode::Svg, PageSetup::Default).unwrap()
    }

    fn html_session() -> TypstReplSession {
        TypstReplSession::new(RenderMode::Html, PageSetup::Default).unwrap()
    }

    fn svg_output(result: ExecutionResult) -> String {
        match result.output {
            ExecutionOutput::Svg(svg) => svg,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn html_output(result: ExecutionResult) -> String {
        match result.output {
            ExecutionOutput::Html(html) => html,
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
}
