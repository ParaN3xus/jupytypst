use std::collections::HashMap;

use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term::termcolor::NoColor;
use codespan_reporting::term::{self, Config};
use ecow::EcoVec;
use typsess::{DiagnosticSource, ExecutionOutput};
use typst::diag::{Severity, SourceDiagnostic};
use typst::syntax::FileId;

pub fn execution_output_to_html(
    output: ExecutionOutput,
) -> Result<String, EcoVec<SourceDiagnostic>> {
    match output {
        ExecutionOutput::Paged(document) => Ok(svg_pages_html(&document)),
        ExecutionOutput::Html(document) => typst_html::html(&document),
    }
}

pub fn format_diagnostics(diagnostics: EcoVec<SourceDiagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_diagnostics_rich_with_sources(
    diagnostics: EcoVec<SourceDiagnostic>,
    sources: &[DiagnosticSource],
) -> String {
    let mut files = SimpleFiles::new();
    let mut file_ids = HashMap::new();
    for source in sources {
        let file_id = files.add(source.name.clone(), source.source.clone());
        file_ids.insert(source.id, file_id);
    }

    let config = Config {
        tab_width: 2,
        ..Default::default()
    };
    let mut output = Vec::new();
    {
        let mut writer = NoColor::new(&mut output);
        for diagnostic in diagnostics {
            let diag = to_codespan_diagnostic(diagnostic, &file_ids);
            if term::emit(&mut writer, &config, &files, &diag).is_err() {
                break;
            }
        }
    }

    String::from_utf8_lossy(&output).trim_end().to_string()
}

fn svg_pages_html(document: &typst::layout::PagedDocument) -> String {
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

fn to_codespan_diagnostic(
    diagnostic: SourceDiagnostic,
    file_ids: &HashMap<FileId, usize>,
) -> Diagnostic<usize> {
    let mut labels = diagnostic
        .span
        .id()
        .and_then(|id| file_ids.get(&id).copied())
        .zip(diagnostic.span.range())
        .map(|(file_id, range)| Label::primary(file_id, range))
        .into_iter()
        .collect::<Vec<_>>();
    labels.extend(diagnostic.trace.iter().filter_map(|trace| {
        trace
            .span
            .id()
            .and_then(|id| file_ids.get(&id).copied())
            .zip(trace.span.range())
            .map(|(file_id, range)| {
                Label::secondary(file_id, range).with_message(trace.v.to_string())
            })
    }));

    let notes = diagnostic
        .hints
        .iter()
        .map(|hint| format!("hint: {hint}"))
        .collect();

    match diagnostic.severity {
        Severity::Error => Diagnostic::error(),
        Severity::Warning => Diagnostic::warning(),
    }
    .with_message(diagnostic.message.to_string())
    .with_notes(notes)
    .with_labels(labels)
}
