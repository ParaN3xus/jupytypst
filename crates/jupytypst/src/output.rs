use ecow::EcoVec;
use scraper::{Html, Selector};
use typsess::ExecutionOutput;
use typst::diag::SourceDiagnostic;

pub fn execution_output_to_html(
    output: ExecutionOutput,
) -> Result<String, EcoVec<SourceDiagnostic>> {
    match output {
        ExecutionOutput::Paged(document) => Ok(svg_pages_html(&document)),
        ExecutionOutput::Html(document) => typst_html::html(&document),
    }
}

pub fn execution_output_to_cli_html(
    output: ExecutionOutput,
    full_html: bool,
) -> Result<String, EcoVec<SourceDiagnostic>> {
    let html = execution_output_to_html(output)?;
    if full_html {
        return Ok(html);
    }
    Ok(body_inner_html(&html).unwrap_or(html))
}

pub fn format_diagnostics(diagnostics: EcoVec<SourceDiagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_diagnostics_rich(diagnostics: EcoVec<SourceDiagnostic>, source: &str) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| format_diagnostic_rich(&diagnostic, source))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_inner_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("body").ok()?;
    document
        .select(&selector)
        .next()
        .map(|body| body.inner_html().trim().to_string())
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

fn format_diagnostic_rich(diagnostic: &SourceDiagnostic, source: &str) -> String {
    let severity = format!("{:?}", diagnostic.severity).to_lowercase();
    let Some(range) = diagnostic.span.range() else {
        return format!("{severity}: {}", diagnostic.message);
    };
    let (line_index, column) = line_column(source, range.start);
    let line = source.lines().nth(line_index).unwrap_or_default();
    let line_number = line_index + 1;
    let start = range.start.min(source.len());
    let line_end = start + source[start..].find('\n').unwrap_or(source.len() - start);
    let end = range.end.min(source.len()).min(line_end).max(start);
    let caret_width = usize::max(1, source[start..end].chars().count());
    let gutter = line_number.to_string();
    let padding = " ".repeat(gutter.len());
    let caret_padding = " ".repeat(column);
    let carets = "^".repeat(caret_width);

    format!(
        "{severity}: {}\n  {padding}┌─ <stdin>:{line_number}:{}\n  {padding}│\n{line_number} │ {line}\n  {padding}│ {caret_padding}{carets}",
        diagnostic.message,
        column + 1,
    )
}

fn line_column(source: &str, byte_index: usize) -> (usize, usize) {
    let mut line = 0;
    let mut line_start = 0;
    for (index, ch) in source.char_indices() {
        if index >= byte_index {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = index + ch.len_utf8();
        }
    }
    let column = source[line_start..byte_index.min(source.len())]
        .chars()
        .count();
    (line, column)
}

#[cfg(test)]
mod tests {
    use ecow::eco_vec;
    use typst::diag::SourceDiagnostic;
    use typst::syntax::{Span, VirtualPath};

    use super::*;

    #[test]
    fn extracts_body_inner_html() {
        let html = "<!DOCTYPE html><html><head></head><body><p>test</p></body></html>";
        assert_eq!(body_inner_html(html).unwrap(), "<p>test</p>");
    }

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
    }
}
