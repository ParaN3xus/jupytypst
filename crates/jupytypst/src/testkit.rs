use std::fmt::Write as _;

use anyhow::Result;
use tempfile::TempDir;
use typsess::{ExecutionOutput, RenderMode, SourceMode, TypstReplSession, WorldOptions};

use crate::DEFAULT_PAGE_SETUP;
use crate::cell::parse_cell;
use crate::output::{execution_output_to_html, format_diagnostics_rich_with_sources};
use crate::session::create_session;

pub struct KernelCase<'a> {
    pub init: KernelInit,
    pub fixtures: Vec<Fixture<'a>>,
    pub requests: Vec<Request<'a>>,
}

pub struct KernelInit {
    pub default_format: RenderMode,
    pub source_mode: SourceMode,
    pub page_setup: String,
    pub world_options: WorldOptions,
}

pub struct Fixture<'a> {
    pub path: &'a str,
    pub contents: &'a str,
}

pub enum Request<'a> {
    Execute { input: &'a str },
    ExecuteRaw { input: &'a str, format: RenderMode },
}

impl Default for KernelInit {
    fn default() -> Self {
        Self {
            default_format: RenderMode::Svg,
            source_mode: SourceMode::Markup,
            page_setup: DEFAULT_PAGE_SETUP.to_string(),
            world_options: WorldOptions::default(),
        }
    }
}

impl KernelInit {
    pub fn new(default_format: RenderMode, source_mode: SourceMode) -> Self {
        Self {
            default_format,
            source_mode,
            ..Self::default()
        }
    }

    pub fn with_page_setup(mut self, page_setup: impl Into<String>) -> Self {
        self.page_setup = page_setup.into();
        self
    }

    pub fn with_world_options(mut self, world_options: WorldOptions) -> Self {
        self.world_options = world_options;
        self
    }
}

pub fn run_case(case: KernelCase<'_>) -> String {
    run_case_result(case).expect("kernel case should run")
}

fn run_case_result(mut case: KernelCase<'_>) -> Result<String> {
    let fixture_root = if case.fixtures.is_empty() {
        None
    } else {
        let root = TempDir::new()?;
        for fixture in &case.fixtures {
            let path = root.path().join(fixture.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, fixture.contents)?;
        }
        case.init.world_options.root = Some(root.path().to_path_buf());
        Some(root)
    };

    let root_display = case
        .init
        .world_options
        .root
        .as_deref()
        .or_else(|| fixture_root.as_ref().map(|root| root.path()))
        .map(|path| path.display().to_string());
    let mut session = create_session(
        case.init.default_format,
        case.init.source_mode,
        case.init.page_setup.clone(),
        case.init.world_options.clone(),
    )
    .map_err(|diagnostics| anyhow::anyhow!("{diagnostics:?}"))?;

    let mut snapshot = String::new();
    write_init(&mut snapshot, &case);
    for (index, request) in case.requests.iter().enumerate() {
        writeln!(snapshot, "\n--- request {} ---", index + 1).unwrap();
        match request {
            Request::Execute { input } => {
                write_execute(&mut snapshot, &mut session, case.init.default_format, input);
            }
            Request::ExecuteRaw { input, format } => {
                write_execute_raw(&mut snapshot, &mut session, input, *format);
            }
        }
    }

    Ok(normalize_snapshot(snapshot, root_display.as_deref()))
}

fn write_init(snapshot: &mut String, case: &KernelCase<'_>) {
    writeln!(snapshot, "init:").unwrap();
    writeln!(snapshot, "  default_format: {:?}", case.init.default_format).unwrap();
    writeln!(snapshot, "  source_mode: {:?}", case.init.source_mode).unwrap();
    write_block(snapshot, "  page_setup", "typc", &case.init.page_setup);
    writeln!(snapshot, "  inputs: {:?}", case.init.world_options.inputs).unwrap();
    if !case.fixtures.is_empty() {
        writeln!(snapshot, "fixtures:").unwrap();
        for fixture in &case.fixtures {
            writeln!(snapshot, "  - path: {}", fixture.path).unwrap();
            write_block(snapshot, "    contents", "typ", fixture.contents);
        }
    }
}

fn write_execute(
    snapshot: &mut String,
    session: &mut TypstReplSession,
    default_format: RenderMode,
    input: &str,
) {
    writeln!(snapshot, "kind: execute").unwrap();
    write_block(snapshot, "input", "typ", input);
    match parse_cell(input, default_format) {
        Ok(cell) => {
            writeln!(snapshot, "effective_format: {:?}", cell.mode).unwrap();
            write_execution_result(snapshot, session, &cell.body, cell.mode);
        }
        Err(error) => {
            writeln!(snapshot, "status: error").unwrap();
            write_block(snapshot, "traceback", "text", &error.to_string());
        }
    }
}

fn write_execute_raw(
    snapshot: &mut String,
    session: &mut TypstReplSession,
    input: &str,
    format: RenderMode,
) {
    writeln!(snapshot, "kind: execute_raw").unwrap();
    writeln!(snapshot, "format: {format:?}").unwrap();
    write_block(snapshot, "input", "typ", input);
    write_execution_result(snapshot, session, input, format);
}

fn write_execution_result(
    snapshot: &mut String,
    session: &mut TypstReplSession,
    input: &str,
    format: RenderMode,
) {
    match session.execute_with_mode(input, format) {
        Ok(result) => {
            writeln!(snapshot, "status: ok").unwrap();
            if result.warnings.is_empty() {
                writeln!(snapshot, "warnings: []").unwrap();
            } else {
                writeln!(snapshot, "warnings:").unwrap();
                for warning in result.warnings {
                    writeln!(snapshot, "  - {}", warning.message).unwrap();
                }
            }
            match result.output {
                ExecutionOutput::Paged(_) => writeln!(snapshot, "output_kind: svg").unwrap(),
                ExecutionOutput::Html(_) => writeln!(snapshot, "output_kind: html").unwrap(),
            }
            match execution_output_to_html(result.output) {
                Ok(output) => write_block(snapshot, "output", "html", &output),
                Err(diagnostics) => {
                    let traceback = format_diagnostics_rich_with_sources(
                        diagnostics,
                        session.diagnostic_sources(),
                    );
                    write_block(snapshot, "traceback", "text", &traceback);
                }
            }
        }
        Err(diagnostics) => {
            writeln!(snapshot, "status: error").unwrap();
            let traceback =
                format_diagnostics_rich_with_sources(diagnostics, session.diagnostic_sources());
            write_block(snapshot, "traceback", "text", &traceback);
        }
    }
}

fn write_block(snapshot: &mut String, label: &str, info: &str, content: &str) {
    writeln!(snapshot, "{label}: |").unwrap();
    writeln!(snapshot, "  ```{info}").unwrap();
    for line in content.lines() {
        writeln!(snapshot, "  {line}").unwrap();
    }
    if content.ends_with('\n') {
        writeln!(snapshot).unwrap();
    }
    writeln!(snapshot, "  ```").unwrap();
}

fn normalize_snapshot(mut snapshot: String, root: Option<&str>) -> String {
    snapshot = snapshot.replace("\r\n", "\n");
    if let Some(root) = root {
        snapshot = snapshot.replace(&root.replace('\\', "/"), "<ROOT>");
        snapshot = snapshot.replace(root, "<ROOT>");
    }
    normalize_workspace_packages(snapshot)
}

fn normalize_workspace_packages(mut snapshot: String) -> String {
    while let Some(start) = snapshot.find("@ws/p") {
        let digits_start = start + "@ws/p".len();
        let digits_len = snapshot[digits_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .map(char::len_utf8)
            .sum::<usize>();
        let version_start = digits_start + digits_len;
        if digits_len == 0 || !snapshot[version_start..].starts_with(":0.0.0") {
            break;
        }
        snapshot.replace_range(start..version_start, "@ws/root");
    }
    snapshot
}
