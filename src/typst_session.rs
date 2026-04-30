use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use comemo::Track;
use ecow::{EcoVec, eco_format};
use parking_lot::Mutex;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::engine::Sink;
use typst::foundations::{Bytes, Datetime, Repr, Scope};
use typst::layout::{Abs, PagedDocument};
use typst::syntax::{FileId, Source, SyntaxMode, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_kit::fonts::{FontSlot, Fonts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Eval,
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
    PlainText(String),
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
            mode: Mode::Eval,
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
            Mode::Eval => self.eval(&cell.body)?,
            Mode::Svg => self.render_svg(&cell.body)?,
            Mode::Html => self.render_html(&cell.body)?,
        };
        self.context_code
            .extend(extract_context_code(&cell.body, cell.mode));
        Ok(output)
    }

    fn eval(&self, code: &str) -> Result<ExecutionOutput> {
        let full_code = join_code(&self.context_code, code);
        let world = SessionWorld::new(&self.root, "", self.fonts.clone_for_world());
        let mut sink = Sink::new();
        let value = typst_eval::eval_string(
            &typst::ROUTINES,
            (&world as &dyn World).track(),
            sink.track_mut(),
            &full_code,
            typst::syntax::Span::detached(),
            SyntaxMode::Code,
            Scope::new(),
        )
        .map_err(format_diagnostics)?;
        Ok(ExecutionOutput::PlainText(value.repr().to_string()))
    }

    fn render_svg(&self, markup: &str) -> Result<ExecutionOutput> {
        let source = self.render_source(markup);
        let world = SessionWorld::new(&self.root, &source, self.fonts.clone_for_world());
        let warned = typst::compile::<PagedDocument>(&world);
        let document = warned.output.map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Svg(typst_svg::svg_merged(
            &document,
            Abs::pt(0.0),
        )))
    }

    fn render_html(&self, markup: &str) -> Result<ExecutionOutput> {
        let source = self.render_source(markup);
        let world = SessionWorld::new(&self.root, &source, self.fonts.clone_for_world());
        let warned = typst::compile::<typst_html::HtmlDocument>(&world);
        let document = warned.output.map_err(format_diagnostics)?;
        Ok(ExecutionOutput::Html(
            typst_html::html(&document).map_err(format_diagnostics)?,
        ))
    }

    fn render_source(&self, markup: &str) -> String {
        let mut source = String::new();
        if let Some(page_setup) = self.page_setup.code() {
            source.push('#');
            source.push_str(page_setup);
            source.push('\n');
        }
        for line in &self.context_code {
            source.push('#');
            source.push_str(line);
            source.push('\n');
        }
        source.push_str(markup);
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
        "eval" => Ok(Mode::Eval),
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
                Mode::Eval => trimmed,
                Mode::Svg | Mode::Html => trimmed.strip_prefix('#')?.trim_start(),
            };
            is_context_statement(code).then(|| code.to_string())
        })
        .collect()
}

fn is_context_statement(code: &str) -> bool {
    ["let ", "set ", "show ", "import ", "include "]
        .iter()
        .any(|prefix| code.starts_with(prefix))
}

fn join_code(context: &[String], code: &str) -> String {
    let mut full = context.join("\n");
    if !full.is_empty() {
        full.push('\n');
    }
    full.push_str(code);
    full
}

fn format_diagnostics(diagnostics: EcoVec<SourceDiagnostic>) -> anyhow::Error {
    let message = diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    anyhow!(message)
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
        let cell = parse_cell("// jupytypst: mode=svg\n= Test", Mode::Eval).unwrap();
        assert_eq!(cell.mode, Mode::Svg);
        assert_eq!(cell.body, "= Test");
    }

    #[test]
    fn extracts_eval_context() {
        let context = extract_context_code("let f(a, b) = a + b\nf(1, 2)", Mode::Eval);
        assert_eq!(context, vec!["let f(a, b) = a + b"]);
    }

    #[test]
    fn extracts_markup_context_without_visible_content() {
        let context = extract_context_code("#let x = 1\n#lorem(100)\n= Test", Mode::Svg);
        assert_eq!(context, vec!["let x = 1"]);
    }

    #[test]
    fn eval_mode_preserves_definitions() {
        let mut session = TypstSession::default();
        let first = session.execute("let f(a, b) = a + b").unwrap();
        assert!(matches!(first, ExecutionOutput::PlainText(_)));

        let second = session.execute("f(1, 2)").unwrap();
        match second {
            ExecutionOutput::PlainText(text) => assert_eq!(text, "3"),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn svg_mode_does_not_rerender_previous_visible_content() {
        let mut session = TypstSession::default();
        session
            .execute("// jupytypst: mode=svg\n#lorem(20)")
            .unwrap();
        assert!(session.context_code.is_empty());

        let output = session.execute("// jupytypst: mode=svg\n= Test").unwrap();
        match output {
            ExecutionOutput::Svg(svg) => {
                assert!(svg.contains("<svg"));
                assert!(!svg.contains("Lorem"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn page_setup_default_is_injected() {
        let session = TypstSession::default();
        let source = session.render_source("= Test");
        assert!(source.starts_with("#set page(width: auto, height: auto, margin: 16pt)"));
    }

    #[test]
    fn page_setup_none_is_not_injected() {
        let session = TypstSession::new(PageSetup::None);
        let source = session.render_source("= Test");
        assert_eq!(source, "= Test");
    }

    #[test]
    fn page_setup_custom_is_injected() {
        let session = TypstSession::new(PageSetup::Custom("set page(paper: \"a4\")".into()));
        let source = session.render_source("= Test");
        assert!(source.starts_with("#set page(paper: \"a4\")"));
    }
}
