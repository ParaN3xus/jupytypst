use std::path::PathBuf;
use std::sync::Arc;

use comemo::Track;
use ecow::{EcoVec, eco_vec};
use tinymist_world::system::TypstSystemWorld;
use tinymist_world::{EntryState, ShadowApi, TaskInputs};
use typst::World;
use typst::diag::SourceDiagnostic;
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{Bytes, Content, Context, Scope, Scopes, Styles};
use typst::introspection::Introspector;
use typst::layout::PagedDocument;
use typst::syntax::{Span, VirtualPath};
use typst_eval::Vm;

mod diagnostics;
mod eval;
mod input;
mod layout;
mod world;

pub use diagnostics::DiagnosticSource;
use diagnostics::{DiagnosticSourceMap, diagnostic_source_name, remap_diagnostics};
use eval::{EvaluatedCell, EvaluatedSource, eval_source_capture, parsed_source, span_offset};
pub use input::{InputStatus, classify_input};
use layout::layout_current_document;
use world::create_world;

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
    pub diagnostic_sources: Vec<DiagnosticSource>,
    pub diagnostic_source_maps: Vec<DiagnosticSourceMap>,
    pub next_source_index: usize,
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
    diagnostic_sources: Vec<DiagnosticSource>,
    diagnostic_source_maps: Vec<DiagnosticSourceMap>,
    next_source_index: usize,
    persistence: SessionPersistence,
    root: PathBuf,
    world: TypstSystemWorld,
}

impl TypstReplSession {
    pub fn new(options: SessionOptions) -> typst::diag::SourceResult<Self> {
        let (world, root) = create_world(&options.world_options)?;
        Ok(Self {
            render_mode: options.render_mode,
            source_mode: options.source_mode,
            scope: options.state.scope,
            styles: options.state.styles,
            introspection_updates: options.state.introspection_updates,
            diagnostic_sources: options.state.diagnostic_sources,
            diagnostic_source_maps: options.state.diagnostic_source_maps,
            next_source_index: options.state.next_source_index,
            persistence: options.persistence,
            root,
            world,
        })
    }

    pub fn into_state(self) -> SessionState {
        SessionState {
            scope: self.scope,
            styles: self.styles,
            introspection_updates: self.introspection_updates,
            diagnostic_sources: self.diagnostic_sources,
            diagnostic_source_maps: self.diagnostic_source_maps,
            next_source_index: self.next_source_index,
        }
    }

    pub fn diagnostic_sources(&self) -> &[DiagnosticSource] {
        &self.diagnostic_sources
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
            RenderMode::Svg => self.render_svg(content),
            RenderMode::Html => self.render_html(content),
        };
        let output = match output {
            Ok(output) => output,
            Err(diagnostics) => {
                let diagnostics = self.prepare_diagnostics(diagnostics, evaluated.source_map_index);
                return Err(diagnostics);
            }
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
            source_map_index: evaluated.source_map_index,
        })
    }

    fn evaluate_source(
        &mut self,
        source: &str,
        source_mode: SourceMode,
        filter_styles: &dyn Fn(Styles) -> Styles,
    ) -> typst::diag::SourceResult<EvaluatedSource> {
        let source_index = self.next_source_index;
        self.next_source_index += 1;
        let source_path = format!(".jupytypst-input-{source_index}.typ");
        let source_file = self.root.join(&source_path);
        let entry = EntryState::new_rooted(
            self.root.clone().into(),
            Some(VirtualPath::new(format!("/{source_path}"))),
        );
        let source_id = entry
            .main()
            .expect("entry source should have a main file id");
        let display_id = typst::syntax::FileId::new_fake(VirtualPath::new(format!(
            "/jupytypst-input-{source_index}.typ"
        )));

        let parsed_source = parsed_source(source_id, source, source_mode);
        self.world = self.world.task(TaskInputs {
            entry: Some(entry),
            inputs: None,
        });
        self.world
            .map_shadow(
                &source_file,
                Bytes::from_string(parsed_source.text().to_string()),
            )
            .map_err(|error| {
                source_error(format!("failed to update Typst source shadow: {error}"))
            })?;
        self.diagnostic_sources.push(DiagnosticSource {
            id: display_id,
            name: format!("<stdin:{}>", source_index + 1),
            source: source.to_string(),
        });
        self.diagnostic_source_maps.push(DiagnosticSourceMap {
            source: parsed_source.clone(),
            display_id,
            offset: span_offset(source_mode),
            source_len: source.len(),
        });
        let source_map_index = self.diagnostic_source_maps.len() - 1;

        let root = parsed_source.root();

        let errors = root.errors();
        if !errors.is_empty() {
            let diagnostics = self.prepare_diagnostics(
                errors.into_iter().map(Into::into).collect(),
                source_map_index,
            );
            return Err(diagnostics);
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
                    let diagnostics = self.prepare_diagnostics(diagnostics, source_map_index);
                    return Err(diagnostics);
                }
            };
            if let Some(flow) = vm.flow {
                let diagnostics =
                    self.prepare_diagnostics(eco_vec![flow.forbidden()], source_map_index);
                return Err(diagnostics);
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
            warnings: remap_diagnostics(
                sink_warnings,
                &self.diagnostic_source_maps,
                source_map_index,
            ),
            source_map_index,
        })
    }

    fn prepare_diagnostics(
        &mut self,
        diagnostics: EcoVec<SourceDiagnostic>,
        primary_source_map_index: usize,
    ) -> EcoVec<SourceDiagnostic> {
        let diagnostics = remap_diagnostics(
            diagnostics,
            &self.diagnostic_source_maps,
            primary_source_map_index,
        );
        self.record_world_diagnostic_sources(&diagnostics);
        remap_diagnostics(
            diagnostics,
            &self.diagnostic_source_maps,
            primary_source_map_index,
        )
    }

    fn record_world_diagnostic_sources(&mut self, diagnostics: &EcoVec<SourceDiagnostic>) {
        let ids = diagnostics
            .iter()
            .flat_map(|diagnostic| {
                std::iter::once(diagnostic.span)
                    .chain(diagnostic.trace.iter().map(|trace| trace.span))
            })
            .filter_map(Span::id)
            .collect::<Vec<_>>();

        for id in ids {
            if self.diagnostic_sources.iter().any(|source| source.id == id) {
                continue;
            }
            let Ok(source) = self.world.source(id) else {
                continue;
            };
            self.diagnostic_sources.push(DiagnosticSource {
                id,
                name: diagnostic_source_name(id),
                source: source.text().to_string(),
            });
            self.diagnostic_source_maps.push(DiagnosticSourceMap {
                source: source.clone(),
                display_id: id,
                offset: 0,
                source_len: source.text().len(),
            });
        }
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

#[cfg(test)]
mod tests;

fn source_error(message: impl Into<ecow::EcoString>) -> EcoVec<SourceDiagnostic> {
    eco_vec![SourceDiagnostic::error(Span::detached(), message)]
}
