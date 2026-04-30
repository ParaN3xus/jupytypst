use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use ecow::{EcoVec, eco_format};
use parking_lot::Mutex;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_kit::fonts::{FontSlot, Fonts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Svg,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSetup {
    Default,
    None,
    Custom(String),
}

impl PageSetup {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "default" => Ok(Self::Default),
            "none" => Ok(Self::None),
            "" => Err(anyhow!("page setup cannot be empty")),
            custom => Ok(Self::Custom(custom.to_string())),
        }
    }

    fn code(&self) -> Option<&str> {
        match self {
            Self::Default => Some("set page(width: auto, height: auto, margin: 16pt)"),
            Self::None => None,
            Self::Custom(code) => Some(code.as_str()),
        }
    }
}

#[derive(Debug)]
pub struct ParsedCell {
    pub mode: Mode,
    pub body: String,
}

#[derive(Debug)]
pub enum ExecutionOutput {
    Svg(String),
    Html(String),
}

#[derive(Debug)]
pub struct TypstSession {
    mode: Mode,
    page_setup: PageSetup,
    context_code: Vec<String>,
    root: PathBuf,
    fonts: Fonts,
}

impl TypstSession {
    pub fn new(page_setup: PageSetup) -> Self {
        let mut searcher = Fonts::searcher();
        let fonts = searcher.search();
        Self {
            mode: Mode::Svg,
            page_setup,
            context_code: Vec::new(),
            root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            fonts,
        }
    }

    pub fn execute(&mut self, source: &str) -> Result<ExecutionOutput> {
        let cell = parse_cell(source, self.mode)?;
        self.mode = cell.mode;
        let output = match cell.mode {
            Mode::Svg => self.render_svg(&cell.body)?,
            Mode::Html => self.render_html(&cell.body)?,
        };
        self.context_code
            .extend(extract_context_code(&cell.body, cell.mode));
        Ok(output)
    }

    fn render_svg(&self, code: &str) -> Result<ExecutionOutput> {
        let source = self.render_source(code);
        let world = SessionWorld::new(&self.root, &source, self.fonts.clone_for_world());
        let warned = typst::compile::<PagedDocument>(&world);
        let document = warned.output.map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Svg(svg_pages_html(&document)))
    }

    fn render_html(&self, code: &str) -> Result<ExecutionOutput> {
        let source = self.render_source(code);
        let world = SessionWorld::new(&self.root, &source, self.fonts.clone_for_world());
        let warned = typst::compile::<typst_html::HtmlDocument>(&world);
        let document = warned.output.map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Html(
            typst_html::html(&document).map_err(format_diagnostics)?,
        ))
    }

    fn render_source(&self, code: &str) -> String {
        let mut source = String::from("#{\n");
        if let Some(page_setup) = self.page_setup.code() {
            source.push_str(normalize_code_statement(page_setup));
            source.push('\n');
        }
        for line in &self.context_code {
            source.push_str(line);
            source.push('\n');
        }
        source.push_str(code);
        source.push_str("\n}\n");
        source
    }
}

impl Default for TypstSession {
    fn default() -> Self {
        Self::new(PageSetup::Default)
    }
}

pub fn parse_cell(source: &str, default_mode: Mode) -> Result<ParsedCell> {
    let mut mode = default_mode;
    let mut body_start = 0;

    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            body_start += line.len();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("// jupytypst:") {
            mode = parse_directive(rest)?;
            body_start += line.len();
            continue;
        }
        break;
    }

    Ok(ParsedCell {
        mode,
        body: source[body_start..].to_string(),
    })
}

fn parse_directive(rest: &str) -> Result<Mode> {
    let rest = rest.trim();
    let Some(value) = rest.strip_prefix("mode=").map(str::trim) else {
        return Err(anyhow!("unsupported jupytypst directive `{rest}`"));
    };
    match value {
        "eval" => Err(anyhow!(
            "jupytypst no longer supports mode=eval; use mode=svg or mode=html"
        )),
        "svg" => Ok(Mode::Svg),
        "html" => Ok(Mode::Html),
        other => Err(anyhow!("unsupported jupytypst mode `{other}`")),
    }
}

fn extract_context_code(source: &str, mode: Mode) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let code = match mode {
                Mode::Svg | Mode::Html => normalize_code_statement(trimmed),
            };
            is_context_statement(code).then(|| code.to_string())
        })
        .collect()
}

fn normalize_code_statement(code: &str) -> &str {
    code.trim_start_matches('#').trim_start()
}

fn is_context_statement(code: &str) -> bool {
    if code.starts_with("set page") {
        return false;
    }
    ["let ", "set ", "show ", "import ", "include "]
        .iter()
        .any(|prefix| code.starts_with(prefix))
}

fn format_diagnostics(diagnostics: EcoVec<SourceDiagnostic>) -> anyhow::Error {
    let message = diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    anyhow!(message)
}

fn svg_pages_html(document: &PagedDocument) -> String {
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

#[derive(Debug)]
struct WorldFonts {
    book: FontBook,
    fonts: Vec<FontSlot>,
}

trait CloneForWorld {
    fn clone_for_world(&self) -> WorldFonts;
}

impl CloneForWorld for Fonts {
    fn clone_for_world(&self) -> WorldFonts {
        let mut searcher = Fonts::searcher();
        let fonts = searcher.search();
        WorldFonts {
            book: fonts.book,
            fonts: fonts.fonts,
        }
    }
}

struct SessionWorld {
    root: PathBuf,
    main: FileId,
    source: Source,
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
    files: Mutex<std::collections::HashMap<FileId, Bytes>>,
}

impl SessionWorld {
    fn new(root: &Path, source: &str, fonts: WorldFonts) -> Self {
        let main = FileId::new_fake(VirtualPath::new("/main.typ"));
        let library = Library::builder()
            .with_features(Features::from_iter([Feature::Html]))
            .build();
        Self {
            root: root.to_path_buf(),
            main,
            source: Source::new(main, source.to_string()),
            library: LazyHash::new(library),
            book: LazyHash::new(fonts.book),
            fonts: fonts.fonts,
            files: Mutex::new(Default::default()),
        }
    }

    fn resolve(&self, id: FileId) -> FileResult<PathBuf> {
        id.vpath()
            .resolve(&self.root)
            .ok_or_else(|| FileError::Other(Some(eco_format!("path escapes project root"))))
    }
}

impl World for SessionWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            return Ok(self.source.clone());
        }
        let path = self.resolve(id)?;
        let text =
            std::fs::read_to_string(&path).map_err(|error| FileError::from_io(error, &path))?;
        Ok(Source::new(id, text))
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if let Some(bytes) = self.files.lock().get(&id) {
            return Ok(bytes.clone());
        }
        let path = self.resolve(id)?;
        let bytes =
            Bytes::new(std::fs::read(&path).map_err(|error| FileError::from_io(error, &path))?);
        self.files.lock().insert(id, bytes.clone());
        Ok(bytes)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index)?.get()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comment_mode_directive() {
        let cell = parse_cell("// jupytypst: mode=svg\n[Test]", Mode::Svg).unwrap();
        assert_eq!(cell.mode, Mode::Svg);
        assert_eq!(cell.body, "[Test]");
    }

    #[test]
    fn rejects_eval_mode() {
        let error = parse_cell("// jupytypst: mode=eval\n1 + 2", Mode::Svg).unwrap_err();
        assert!(error.to_string().contains("mode=eval"));
    }

    #[test]
    fn extracts_code_context_without_visible_content() {
        let context = extract_context_code(
            "let x = 1\nset page(paper: \"a4\")\nset text(size: 14pt)\nlorem(100)\n[Test]",
            Mode::Svg,
        );
        assert_eq!(context, vec!["let x = 1", "set text(size: 14pt)"]);
    }

    #[test]
    fn default_mode_is_svg() {
        let cell = parse_cell("[Test]", Mode::Svg).unwrap();
        assert_eq!(cell.mode, Mode::Svg);
    }

    #[test]
    fn svg_mode_does_not_rerender_previous_visible_content() {
        let mut session = TypstSession::default();
        session
            .execute("// jupytypst: mode=svg\nlorem(20)")
            .unwrap();
        assert!(session.context_code.is_empty());

        let output = session.execute("// jupytypst: mode=svg\n[Test]").unwrap();
        match output {
            ExecutionOutput::Svg(svg) => {
                assert!(svg.contains("<svg"));
                assert!(!svg.contains("Lorem"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn code_context_persists_without_hash_prefix() {
        let mut session = TypstSession::default();
        session.execute("let f(a, b) = a + b").unwrap();
        let output = session.execute("f(1, 2)").unwrap();
        match output {
            ExecutionOutput::Svg(html) => assert!(html.contains("<svg")),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn page_set_rules_do_not_persist_between_cells() {
        let mut session = TypstSession::default();
        session.execute("set page(paper: \"a4\")\n[First]").unwrap();

        let source = session.render_source("[Second]");
        assert!(source.contains("set page(width: auto, height: auto, margin: 16pt)"));
        assert!(!source.contains("paper: \"a4\""));
    }

    #[test]
    fn page_setup_default_is_injected() {
        let session = TypstSession::default();
        let source = session.render_source("[Test]");
        assert!(source.starts_with("#{\nset page(width: auto, height: auto, margin: 16pt)"));
    }

    #[test]
    fn page_setup_none_is_not_injected() {
        let session = TypstSession::new(PageSetup::None);
        let source = session.render_source("[Test]");
        assert_eq!(source, "#{\n[Test]\n}\n");
    }

    #[test]
    fn page_setup_custom_is_injected() {
        let session = TypstSession::new(PageSetup::Custom("#set page(paper: \"a4\")".into()));
        let source = session.render_source("[Test]");
        assert!(source.starts_with("#{\nset page(paper: \"a4\")"));
    }

    #[test]
    fn svg_mode_wraps_multiple_pages_as_independent_svgs() {
        let mut session = TypstSession::default();
        let output = session
            .execute("// jupytypst: mode=svg\n[x]\n\npagebreak()\n\n[x]")
            .unwrap();
        match output {
            ExecutionOutput::Svg(html) => {
                assert!(html.contains("jupytypst-pages"));
                assert!(html.matches("<svg").count() >= 2);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }
}
