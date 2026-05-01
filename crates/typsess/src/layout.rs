use comemo::{Constraint, Track};
use typst::World;
use typst::diag::{SourceResult, Warned, warning};
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{Content, StyleChain, Target, TargetElem};
use typst::introspection::Introspector;
use typst::layout::PagedDocument;
use typst::syntax::Span;

pub(crate) fn layout_current_document<D: LayoutTarget>(
    world: &dyn World,
    content: &Content,
) -> SourceResult<Warned<D>> {
    let library = world.library();
    let base = StyleChain::new(&library.styles);
    let target_style = TargetElem::target.set(D::TARGET).wrap();
    let styles = base.chain(&target_style);
    let empty_introspector = Introspector::default();
    let traced = Traced::default();
    let mut introspector = &empty_introspector;
    let mut subsink;
    let mut document;

    for iteration in 0..5 {
        let constraint = Constraint::new();
        subsink = Sink::new();
        document = {
            let mut engine = Engine {
                routines: &typst::ROUTINES,
                world: world.track(),
                introspector: introspector.track_with(&constraint),
                traced: traced.track(),
                sink: subsink.track_mut(),
                route: Route::default(),
            };
            D::layout(&mut engine, content, styles)?
        };
        introspector = document.introspector();

        if constraint.validate(introspector) {
            let delayed = subsink.delayed();
            if !delayed.is_empty() {
                return Err(delayed);
            }
            return Ok(Warned {
                output: document,
                warnings: subsink.warnings(),
            });
        }

        if iteration == 4 {
            subsink.warn(warning!(
                Span::detached(), "layout did not converge within 5 attempts";
                hint: "check if any states or queries are updating themselves"
            ));
            let delayed = subsink.delayed();
            if !delayed.is_empty() {
                return Err(delayed);
            }
            return Ok(Warned {
                output: document,
                warnings: subsink.warnings(),
            });
        }
    }

    unreachable!("layout loop always returns within five iterations")
}

pub(crate) trait LayoutTarget: Sized {
    const TARGET: Target;

    fn layout(engine: &mut Engine, content: &Content, styles: StyleChain) -> SourceResult<Self>;

    fn introspector(&self) -> &Introspector;
}

impl LayoutTarget for PagedDocument {
    const TARGET: Target = Target::Paged;

    fn layout(engine: &mut Engine, content: &Content, styles: StyleChain) -> SourceResult<Self> {
        typst_layout::layout_document(engine, content, styles)
    }

    fn introspector(&self) -> &Introspector {
        &self.introspector
    }
}

impl LayoutTarget for typst_html::HtmlDocument {
    const TARGET: Target = Target::Html;

    fn layout(engine: &mut Engine, content: &Content, styles: StyleChain) -> SourceResult<Self> {
        typst_html::html_document(engine, content, styles)
    }

    fn introspector(&self) -> &Introspector {
        &self.introspector
    }
}
