use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::termcolor::NoColor;
use codespan_reporting::term::{self, Config};
use ecow::EcoVec;
use typsess::ExecutionOutput;
use typst::diag::{Severity, SourceDiagnostic};

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

pub fn format_diagnostics_rich(diagnostics: EcoVec<SourceDiagnostic>, source: &str) -> String {
    let file = SimpleFile::new("<stdin>", source);
    let config = Config {
        tab_width: 2,
        ..Default::default()
    };
    let mut output = Vec::new();
    {
        let mut writer = NoColor::new(&mut output);
        for diagnostic in diagnostics {
            let diag = to_codespan_diagnostic(diagnostic);
            if term::emit(&mut writer, &config, &file, &diag).is_err() {
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

fn to_codespan_diagnostic(diagnostic: SourceDiagnostic) -> Diagnostic<()> {
    let mut labels = diagnostic
        .span
        .range()
        .map(|range| Label::primary((), range))
        .into_iter()
        .collect::<Vec<_>>();
    labels.extend(diagnostic.trace.iter().filter_map(|trace| {
        trace
            .span
            .range()
            .map(|range| Label::secondary((), range).with_message(trace.v.to_string()))
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

#[cfg(test)]
mod tests {
    use ecow::eco_vec;
    use typst::diag::SourceDiagnostic;
    use typst::syntax::{Span, VirtualPath};

    use super::*;

    #[test]
    fn formats_source_diagnostic_with_line_and_caret() {
        let diagnostics = eco_vec![SourceDiagnostic::error(
            Span::from_range(
                typst::syntax::FileId::new(None, VirtualPath::new("/main.typ")),
                0..5,
            ),
            "unknown variable: tests",
        )];

        let formatted = format_diagnostics_rich(diagnostics, "tests\n");
        assert!(formatted.contains("error: unknown variable: tests"));
        assert!(formatted.contains("1 │ tests"));
        assert!(formatted.contains("^^^^^"));
        assert!(formatted.contains("  ┌─ <stdin>:1:1"));
    }

    #[test]
    fn formats_tracepoints_as_secondary_labels() {
        let source = "#let f() = panic()\n\n#let g() = f()\n\n#g()\n";
        let mut diagnostic = SourceDiagnostic::error(
            Span::from_range(
                typst::syntax::FileId::new(None, VirtualPath::new("/main.typ")),
                11..18,
            ),
            "panicked",
        );
        diagnostic.trace = eco_vec![
            typst::syntax::Spanned::new(
                typst::diag::Tracepoint::Call(Some("f".into())),
                Span::from_range(
                    typst::syntax::FileId::new(None, VirtualPath::new("/main.typ")),
                    31..34,
                ),
            ),
            typst::syntax::Spanned::new(
                typst::diag::Tracepoint::Call(Some("g".into())),
                Span::from_range(
                    typst::syntax::FileId::new(None, VirtualPath::new("/main.typ")),
                    39..42,
                ),
            ),
        ];

        let formatted = format_diagnostics_rich(eco_vec![diagnostic], source);
        assert!(formatted.contains("error: panicked"));
        assert!(formatted.contains("error occurred in this call of function `f`"));
        assert!(formatted.contains("error occurred in this call of function `g`"));
    }
}
