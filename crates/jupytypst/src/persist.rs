use std::ops::ControlFlow;

use ecow::eco_vec;
use typst::foundations::{Content, Element, Selector, Styles};
use typst::introspection::{Counter, State};
use typst::layout::PageElem;

pub fn filter_persistent_styles(styles: Styles) -> Styles {
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

pub fn collect_introspection_updates(content: &Content) -> Vec<Content> {
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
