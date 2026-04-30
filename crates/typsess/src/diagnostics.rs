use ecow::EcoVec;
use typst::diag::SourceDiagnostic;
use typst::syntax::{FileId, Source, Span};

#[derive(Debug, Clone)]
pub struct DiagnosticSource {
    pub id: FileId,
    pub name: String,
    pub source: String,
}

#[derive(Clone)]
pub struct DiagnosticSourceMap {
    pub(crate) source: Source,
    pub(crate) display_id: FileId,
    pub(crate) offset: usize,
    pub(crate) source_len: usize,
}

pub(crate) fn diagnostic_source_name(id: FileId) -> String {
    match id.package() {
        Some(package) => format!("{package}{}", id.vpath().as_rooted_path().display()),
        None => id.vpath().as_rooted_path().display().to_string(),
    }
}

pub(crate) fn remap_diagnostics(
    mut diagnostics: EcoVec<SourceDiagnostic>,
    source_maps: &[DiagnosticSourceMap],
    primary_source_map_index: usize,
) -> EcoVec<SourceDiagnostic> {
    for diagnostic in diagnostics.make_mut() {
        diagnostic.span = remap_span(diagnostic.span, source_maps, primary_source_map_index);
        for trace in diagnostic.trace.make_mut() {
            trace.span = remap_span(trace.span, source_maps, primary_source_map_index);
        }
    }
    diagnostics
}

fn remap_span(
    span: Span,
    source_maps: &[DiagnosticSourceMap],
    primary_source_map_index: usize,
) -> Span {
    let primary = source_maps
        .get(primary_source_map_index)
        .and_then(|source_map| remap_span_for_source(span, source_map));
    if let Some(span) = primary {
        return span;
    }

    source_maps
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != primary_source_map_index)
        .find_map(|(_, source_map)| remap_span_for_source(span, source_map))
        .unwrap_or(span)
}

fn remap_span_for_source(span: Span, source_map: &DiagnosticSourceMap) -> Option<Span> {
    let range = source_map.source.range(span)?;

    let end = source_map.offset + source_map.source_len;
    if range.start < source_map.offset || range.end > end {
        return None;
    }

    Some(Span::from_range(
        source_map.display_id,
        range.start - source_map.offset..range.end - source_map.offset,
    ))
}
