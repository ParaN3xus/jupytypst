use std::path::{Path, PathBuf};

use anyhow::anyhow;
use comemo::{Constraint, Track};
use ecow::{EcoVec, eco_vec};
use tinymist_world::args::CompileFontArgs;
use tinymist_world::system::{SystemUniverseBuilder, TypstSystemWorld};
use tinymist_world::{EntryState, ShadowApi};
use typst::World;
use typst::diag::{At, SourceDiagnostic};
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{
    Bytes, Content, Context, Scope, Scopes, Style, StyleChain, Styles, Target, TargetElem, Value,
    ops,
};
use typst::introspection::Introspector;
use typst::layout::PagedDocument;
use typst::syntax::{Span, VirtualPath, ast, ast::AstNode, parse_code};
use typst_eval::{Eval, Vm};

mod input;
mod persist;

pub use input::{InputStatus, classify_input};

use persist::{collect_introspection_updates, filter_persistent_styles};

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
    pub fn parse(value: &str) -> anyhow::Result<Self> {
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
    Paged(PagedDocument),
    Html(typst_html::HtmlDocument),
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub output: ExecutionOutput,
    pub warnings: EcoVec<SourceDiagnostic>,
}

pub struct TypstReplSession {
    render_mode: RenderMode,
    scope: Scope,
    styles: Styles,
    introspection_updates: Vec<Content>,
    world: TypstSystemWorld,
}

impl TypstReplSession {
    pub fn new(render_mode: RenderMode, page_setup: PageSetup) -> typst::diag::SourceResult<Self> {
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

    pub fn execute(&mut self, source: &str) -> typst::diag::SourceResult<ExecutionResult> {
        self.execute_with_mode(source, self.render_mode)
    }

    pub fn execute_with_mode(
        &mut self,
        source: &str,
        render_mode: RenderMode,
    ) -> typst::diag::SourceResult<ExecutionResult> {
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

    fn initialize_page_setup(&mut self, page_setup: PageSetup) -> typst::diag::SourceResult<()> {
        if let Some(page_setup) = page_setup.code() {
            let setup = normalize_code_statement(page_setup);
            let evaluated = self.evaluate_source(setup, StyleCapture::Local)?;
            self.styles.apply(evaluated.captured_styles);
        }
        Ok(())
    }

    fn evaluate_code(&mut self, code: &str) -> typst::diag::SourceResult<EvaluatedCell> {
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
    ) -> typst::diag::SourceResult<EvaluatedSource> {
        self.world
            .map_shadow_by_id(self.world.main(), Bytes::from_string(source.to_string()))
            .map_err(|error| {
                source_error(format!("failed to update Typst main source: {error}"))
            })?;

        let span = Span::from_range(self.world.main(), 0..source.len());
        let mut root = parse_code(source);
        root.synthesize(span);

        let errors = root.errors();
        if !errors.is_empty() {
            return Err(errors.into_iter().map(Into::into).collect());
        }

        let mut sink = Sink::new();
        let mut captured_styles = Styles::new();
        let mut warnings = eco_vec![];
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
            )?;
            if let Some(flow) = vm.flow {
                return Err(eco_vec![flow.forbidden()]);
            }
            let new_scope = vm.scopes.top.clone();
            drop(vm);
            (value, new_scope, sink.warnings())
        };

        warnings.extend(sink_warnings);

        Ok(EvaluatedSource {
            value,
            scope: new_scope,
            captured_styles,
            warnings,
        })
    }

    fn render_svg(&self, content: Content) -> typst::diag::SourceResult<ExecutionOutput> {
        let world = self.world.paged_task();
        let document = layout_paged_document(world.as_ref(), &content)?;
        Ok(ExecutionOutput::Paged(document))
    }

    fn render_html(&self, content: Content) -> typst::diag::SourceResult<ExecutionOutput> {
        let world = self.world.html_task();
        let document = layout_html_document(world.as_ref(), &content)?;
        Ok(ExecutionOutput::Html(document))
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

struct EvaluatedCell {
    content: Content,
    warnings: EcoVec<SourceDiagnostic>,
}

struct EvaluatedSource {
    value: Value,
    scope: Scope,
    captured_styles: Styles,
    warnings: EcoVec<SourceDiagnostic>,
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
    warnings: &mut EcoVec<SourceDiagnostic>,
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
                            warnings.push(SourceDiagnostic::warning(
                                span,
                                "anonymous `show: ...` rules are cell-local and are not persisted",
                            ));
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

fn create_world() -> typst::diag::SourceResult<TypstSystemWorld> {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry = EntryState::new_rooted(
        root.into(),
        Some(VirtualPath::new(Path::new("/__jupytypst__.typ"))),
    );
    let fonts = SystemUniverseBuilder::resolve_fonts(CompileFontArgs::default())
        .map_err(|error| source_error(error.to_string()))?;
    let package_registry = SystemUniverseBuilder::resolve_package(None, None);
    let universe =
        SystemUniverseBuilder::build(entry, Default::default(), fonts.into(), package_registry);
    Ok(universe.snapshot())
}

#[cfg(test)]
mod tests;

fn source_error(message: impl Into<ecow::EcoString>) -> EcoVec<SourceDiagnostic> {
    eco_vec![SourceDiagnostic::error(Span::detached(), message)]
}
