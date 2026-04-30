use comemo::{Constraint, Track};
use typst::World;
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{Content, StyleChain, Target, TargetElem};
use typst::introspection::Introspector;
use typst::layout::PagedDocument;

pub(crate) fn layout_current_document<D: LayoutTarget>(
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

pub(crate) trait LayoutTarget: Sized {
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
