use ecow::EcoVec;
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
    let Some(range) = display_range(diagnostic, source) else {
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
    let caret_padding = " ".repeat(column);
    let carets = "^".repeat(caret_width);
    let marker_indent = " ".repeat(gutter.len() + 1);

    format!(
        "{severity}: {}\n{marker_indent}┌─ <stdin>:{line_number}:{}\n{marker_indent}│\n{line_number} │ {line}\n{marker_indent}│ {caret_padding}{carets}",
        diagnostic.message,
        column + 1,
    )
}

fn display_range(diagnostic: &SourceDiagnostic, source: &str) -> Option<std::ops::Range<usize>> {
    diagnostic_message_unknown_variable(&diagnostic.message)
        .and_then(|name| find_identifier(source, name))
        .or_else(|| diagnostic.span.range())
}

fn diagnostic_message_unknown_variable(message: &str) -> Option<&str> {
    message.strip_prefix("unknown variable: ").map(str::trim)
}

fn find_identifier(source: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let mut matches = source.match_indices(name).filter_map(|(start, matched)| {
        let end = start + matched.len();
        is_identifier_boundary(source, start, end).then_some(start..end)
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn is_identifier_boundary(source: &str, start: usize, end: usize) -> bool {
    let before = source[..start].chars().next_back();
    let after = source[end..].chars().next();
    !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch == '-' || ch.is_alphanumeric()
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
    fn formats_unknown_variable_at_token_in_parenthesized_expression() {
        let diagnostics = eco_vec![SourceDiagnostic::error(
            Span::from_range(
                typst::syntax::FileId::new(None, VirtualPath::new("/main.typ")),
                0..9,
            ),
            "unknown variable: fdsf",
        )];

        let formatted = format_diagnostics_rich(diagnostics, "(\nfdsf\n)\n");
        assert!(formatted.contains("2 │ fdsf"));
        assert!(formatted.contains("  │ ^^^^"));
    }
}
