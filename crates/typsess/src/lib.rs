use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use comemo::{Constraint, Track};
use ecow::{EcoVec, eco_vec};
use tinymist_vfs::ImmutDict;
use tinymist_world::args::CompilePackageArgs;
use tinymist_world::config::CompileFontOpts;
use tinymist_world::font::{FontResolverImpl, system::SystemFontSearcher};
use tinymist_world::system::{SystemUniverseBuilder, TypstSystemWorld};
use tinymist_world::{EntryState, ShadowApi};
use typst::World;
use typst::diag::{At, SourceDiagnostic};
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{
    Bytes, Content, Context, IntoValue, Scope, Scopes, Style, StyleChain, Styles, Target,
    TargetElem, Value, ops,
};
use typst::introspection::Introspector;
use typst::layout::PagedDocument;
use typst::syntax::{Source, Span, VirtualPath, ast, ast::AstNode};
use typst::utils::LazyHash;
use typst_eval::{Eval, Vm};

mod input;

pub use input::{InputStatus, classify_input};

const CODE_WRAPPER_PREFIX: &str = "#{\n";
const CODE_WRAPPER_SUFFIX: &str = "\n}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Svg,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    Code,
    Markup,
}

#[derive(Debug, Clone, Default)]
pub struct WorldOptions {
    pub root: Option<PathBuf>,
    pub inputs: Vec<(String, String)>,
    pub font_paths: Vec<PathBuf>,
    pub ignore_system_fonts: bool,
    pub ignore_embedded_fonts: bool,
    pub package_path: Option<PathBuf>,
    pub package_cache_path: Option<PathBuf>,
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

#[derive(Default)]
pub struct SessionState {
    pub scope: Scope,
    pub styles: Styles,
    pub introspection_updates: Vec<Content>,
}

pub type StylePersistence = Arc<dyn Fn(Styles) -> Styles + Send + Sync + 'static>;
pub type IntrospectionPersistence = Arc<dyn Fn(&Content) -> Vec<Content> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct SessionPersistence {
    pub filter_styles: StylePersistence,
    pub collect_introspection_updates: IntrospectionPersistence,
}

impl Default for SessionPersistence {
    fn default() -> Self {
        Self {
            filter_styles: Arc::new(|styles| styles),
            collect_introspection_updates: Arc::new(|_| Vec::new()),
        }
    }
}

pub struct SessionOptions {
    pub render_mode: RenderMode,
    pub source_mode: SourceMode,
    pub world_options: WorldOptions,
    pub state: SessionState,
    pub persistence: SessionPersistence,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            render_mode: RenderMode::Html,
            source_mode: SourceMode::Markup,
            world_options: WorldOptions::default(),
            state: SessionState::default(),
            persistence: SessionPersistence::default(),
        }
    }
}

pub struct TypstReplSession {
    render_mode: RenderMode,
    source_mode: SourceMode,
    scope: Scope,
    styles: Styles,
    introspection_updates: Vec<Content>,
    persistence: SessionPersistence,
    world: TypstSystemWorld,
}

impl TypstReplSession {
    pub fn new(options: SessionOptions) -> typst::diag::SourceResult<Self> {
        let world = create_world(&options.world_options)?;
        Ok(Self {
            render_mode: options.render_mode,
            source_mode: options.source_mode,
            scope: options.state.scope,
            styles: options.state.styles,
            introspection_updates: options.state.introspection_updates,
            persistence: options.persistence,
            world,
        })
    }

    pub fn into_state(self) -> SessionState {
        SessionState {
            scope: self.scope,
            styles: self.styles,
            introspection_updates: self.introspection_updates,
        }
    }

    pub fn apply_source(
        &mut self,
        source: &str,
        source_mode: SourceMode,
    ) -> typst::diag::SourceResult<()> {
        let evaluated = self.evaluate_source(source, source_mode, &|styles| styles)?;
        self.scope = evaluated.scope;
        self.styles.apply(evaluated.captured_styles);
        Ok(())
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
            .extend((self.persistence.collect_introspection_updates)(
                &evaluated.content,
            ));
        Ok(ExecutionResult {
            output,
            warnings: evaluated.warnings,
        })
    }

    fn evaluate_code(&mut self, code: &str) -> typst::diag::SourceResult<EvaluatedCell> {
        let filter_styles = Arc::clone(&self.persistence.filter_styles);
        let evaluated = self.evaluate_source(code, self.source_mode, filter_styles.as_ref())?;
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
        source_mode: SourceMode,
        filter_styles: &dyn Fn(Styles) -> Styles,
    ) -> typst::diag::SourceResult<EvaluatedSource> {
        self.world
            .map_shadow_by_id(self.world.main(), Bytes::from_string(source.to_string()))
            .map_err(|error| {
                source_error(format!("failed to update Typst main source: {error}"))
            })?;

        let parsed_source = parsed_source(self.world.main(), source, source_mode);
        let root = parsed_source.root();
        let span_offset = span_offset(source_mode);

        let errors = root.errors();
        if !errors.is_empty() {
            return Err(remap_diagnostics(
                errors.into_iter().map(Into::into).collect(),
                &parsed_source,
                span_offset,
                source.len(),
            ));
        }

        let mut sink = Sink::new();
        let mut captured_styles = Styles::new();
        let world = self.world.html_task();
        let evaluated = {
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
            let value = match eval_source_capture(
                &mut vm,
                root,
                source_mode,
                filter_styles,
                &mut captured_styles,
            ) {
                Ok(value) => value,
                Err(diagnostics) => {
                    return Err(remap_diagnostics(
                        diagnostics,
                        &parsed_source,
                        span_offset,
                        source.len(),
                    ));
                }
            };
            if let Some(flow) = vm.flow {
                return Err(remap_diagnostics(
                    eco_vec![flow.forbidden()],
                    &parsed_source,
                    span_offset,
                    source.len(),
                ));
            }
            let new_scope = vm.scopes.top.clone();
            drop(vm);
            (value, new_scope, sink.warnings())
        };
        let (value, new_scope, sink_warnings) = evaluated;

        Ok(EvaluatedSource {
            value,
            scope: new_scope,
            captured_styles,
            warnings: remap_diagnostics(sink_warnings, &parsed_source, span_offset, source.len()),
        })
    }

    fn render_svg(&self, content: Content) -> typst::diag::SourceResult<ExecutionOutput> {
        let world = self.world.paged_task();
        let document = layout_current_document(world.as_ref(), &content)?;
        Ok(ExecutionOutput::Paged(document))
    }

    fn render_html(&self, content: Content) -> typst::diag::SourceResult<ExecutionOutput> {
        let world = self.world.html_task();
        let document = layout_current_document(world.as_ref(), &content)?;
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
        Self::new(SessionOptions::default()).expect("default session options should be valid")
    }
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

fn eval_code_capture<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    let flow = vm.flow.take();
    let mut output = Value::None;

    while let Some(expr) = exprs.next() {
        let value = match expr {
            ast::Expr::SetRule(set) => {
                let styles = set.eval(vm)?;
                captured_styles.apply(filter_styles(styles.clone()));
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, filter_styles, captured_styles)?.display();
                Value::Content(tail.styled_with_map(styles))
            }
            ast::Expr::ShowRule(show) => {
                let recipe = show.eval(vm)?;
                captured_styles.apply(Style::from(recipe.clone()).into());
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, filter_styles, captured_styles)?.display();
                Value::Content(tail.styled_with_recipe(&mut vm.engine, vm.context, recipe)?)
            }
            ast::Expr::CodeBlock(block) => {
                eval_code_block_capture(vm, block, filter_styles, captured_styles)?
            }
            _ => {
                let span = expr.span();
                let value = expr.eval(vm)?;
                output = ops::join(output, value).at(span)?;

                if vm.flow.is_some() {
                    break;
                }
                continue;
            }
        };

        output = ops::join(output, value).at(expr.span())?;

        if vm.flow.is_some() {
            break;
        }
    }

    if flow.is_some() {
        vm.flow = flow;
    }

    Ok(output)
}

fn eval_markup_capture<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Content> {
    let flow = vm.flow.take();
    let mut output = Vec::new();

    while let Some(expr) = exprs.next() {
        match expr {
            ast::Expr::SetRule(set) => {
                let styles = set.eval(vm)?;
                captured_styles.apply(filter_styles(styles.clone()));
                if vm.flow.is_some() {
                    break;
                }
                output.push(
                    eval_markup_capture(vm, exprs, filter_styles, captured_styles)?
                        .styled_with_map(styles),
                );
            }
            ast::Expr::ShowRule(show) => {
                let recipe = show.eval(vm)?;
                captured_styles.apply(Style::from(recipe.clone()).into());
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_markup_capture(vm, exprs, filter_styles, captured_styles)?;
                output.push(tail.styled_with_recipe(&mut vm.engine, vm.context, recipe)?);
            }
            expr => {
                let value = expr.eval(vm)?;
                if !matches!(value, Value::Label(_)) {
                    output.push(value.display().spanned(expr.span()));
                }
            }
        }

        if vm.flow.is_some() {
            break;
        }
    }

    if flow.is_some() {
        vm.flow = flow;
    }

    Ok(Content::sequence(output))
}

fn eval_source_capture(
    vm: &mut Vm,
    root: &typst::syntax::SyntaxNode,
    source_mode: SourceMode,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    match source_mode {
        SourceMode::Code => {
            let code = wrapped_code_body(root)?;
            eval_code_capture(vm, &mut code.exprs(), filter_styles, captured_styles)
        }
        SourceMode::Markup => {
            let markup = root
                .cast::<ast::Markup>()
                .ok_or_else(|| source_error("failed to parse Typst markup"))?;
            Ok(Value::Content(eval_markup_capture(
                vm,
                &mut markup.exprs(),
                filter_styles,
                captured_styles,
            )?))
        }
    }
}

fn eval_code_block_capture(
    vm: &mut Vm,
    block: ast::CodeBlock,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    vm.scopes.enter();
    let output = eval_code_capture(
        vm,
        &mut block.body().exprs(),
        filter_styles,
        captured_styles,
    );
    vm.scopes.exit();
    output
}

fn wrapped_code_body(root: &typst::syntax::SyntaxNode) -> typst::diag::SourceResult<ast::Code<'_>> {
    let markup = root
        .cast::<ast::Markup>()
        .ok_or_else(|| source_error("failed to parse wrapped Typst code"))?;
    markup
        .exprs()
        .find_map(|expr| match expr {
            ast::Expr::CodeBlock(block) => Some(block.body()),
            _ => None,
        })
        .ok_or_else(|| source_error("failed to find wrapped Typst code body"))
}

fn parsed_source(file_id: typst::syntax::FileId, source: &str, mode: SourceMode) -> Source {
    let text = match mode {
        SourceMode::Code => format!("{CODE_WRAPPER_PREFIX}{source}{CODE_WRAPPER_SUFFIX}"),
        SourceMode::Markup => source.to_string(),
    };
    Source::new(file_id, text)
}

fn span_offset(mode: SourceMode) -> usize {
    match mode {
        SourceMode::Code => CODE_WRAPPER_PREFIX.len(),
        SourceMode::Markup => 0,
    }
}

fn remap_diagnostics(
    mut diagnostics: EcoVec<SourceDiagnostic>,
    source: &Source,
    offset: usize,
    source_len: usize,
) -> EcoVec<SourceDiagnostic> {
    for diagnostic in diagnostics.make_mut() {
        diagnostic.span = remap_span(diagnostic.span, source, offset, source_len);
        for trace in diagnostic.trace.make_mut() {
            trace.span = remap_span(trace.span, source, offset, source_len);
        }
    }
    diagnostics
}

fn remap_span(span: Span, source: &Source, offset: usize, source_len: usize) -> Span {
    let Some(range) = source.range(span) else {
        return span;
    };

    let end = offset + source_len;
    if range.start < offset || range.end > end {
        return span;
    }

    Span::from_range(source.id(), range.start - offset..range.end - offset)
}

trait LayoutTarget: Sized {
    const TARGET: Target;

    fn layout(
        engine: &mut Engine,
        content: &Content,
        styles: StyleChain,
    ) -> typst::diag::SourceResult<Self>;

    fn introspector(&self) -> &Introspector;
}

impl LayoutTarget for PagedDocument {
    const TARGET: Target = Target::Paged;

    fn layout(
        engine: &mut Engine,
        content: &Content,
        styles: StyleChain,
    ) -> typst::diag::SourceResult<Self> {
        typst_layout::layout_document(engine, content, styles)
    }

    fn introspector(&self) -> &Introspector {
        &self.introspector
    }
}

impl LayoutTarget for typst_html::HtmlDocument {
    const TARGET: Target = Target::Html;

    fn layout(
        engine: &mut Engine,
        content: &Content,
        styles: StyleChain,
    ) -> typst::diag::SourceResult<Self> {
        typst_html::html_document(engine, content, styles)
    }

    fn introspector(&self) -> &Introspector {
        &self.introspector
    }
}

fn layout_current_document<D: LayoutTarget>(
    world: &dyn World,
    content: &Content,
) -> typst::diag::SourceResult<D> {
    let library = world.library();
    let base = StyleChain::new(&library.styles);
    let target_style = TargetElem::target.set(D::TARGET).wrap();
    let styles = base.chain(&target_style);
    let empty_introspector = Introspector::default();
    let traced = Traced::default();
    let mut previous = None;

    for iteration in 0..5 {
        let current_introspector = previous
            .as_ref()
            .map(LayoutTarget::introspector)
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
            D::layout(&mut engine, content, styles)?
        };

        let delayed = sink.delayed();
        if !delayed.is_empty() {
            return Err(delayed);
        }

        if constraint.validate(document.introspector()) || iteration == 4 {
            return Ok(document);
        }

        previous = Some(document);
    }

    unreachable!("layout loop always returns within five iterations")
}
fn create_world(options: &WorldOptions) -> typst::diag::SourceResult<TypstSystemWorld> {
    let root = options
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(root)
    };
    let entry = EntryState::new_rooted(root.into(), Some(VirtualPath::new(Path::new("/main.typ"))));
    let fonts = resolve_fonts(options).map_err(|error| source_error(error.to_string()))?;
    let package_options = CompilePackageArgs {
        package_path: options.package_path.clone(),
        package_cache_path: options.package_cache_path.clone(),
    };
    let package_registry = SystemUniverseBuilder::resolve_package(None, Some(&package_options));
    let universe = SystemUniverseBuilder::build(
        entry,
        resolve_inputs(options),
        fonts.into(),
        package_registry,
    );
    Ok(universe.snapshot())
}

fn resolve_inputs(options: &WorldOptions) -> ImmutDict {
    let pairs = options
        .inputs
        .iter()
        .map(|(key, value)| (key.as_str().into(), value.as_str().into_value()));
    Arc::new(LazyHash::new(pairs.collect()))
}

fn resolve_fonts(options: &WorldOptions) -> anyhow::Result<FontResolverImpl> {
    let mut searcher = SystemFontSearcher::new();
    let embedded_fonts = if options.ignore_embedded_fonts {
        Vec::new()
    } else {
        typst_assets::fonts().map(Cow::Borrowed).collect()
    };
    searcher.resolve_opts(CompileFontOpts {
        font_paths: options.font_paths.clone(),
        no_system_fonts: options.ignore_system_fonts,
        with_embedded_fonts: embedded_fonts,
    })?;
    Ok(searcher.build())
}

#[cfg(test)]
mod tests;

fn source_error(message: impl Into<ecow::EcoString>) -> EcoVec<SourceDiagnostic> {
    eco_vec![SourceDiagnostic::error(Span::detached(), message)]
}
