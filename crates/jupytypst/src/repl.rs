use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Result, anyhow};
use reedline::{
    Emacs, KeyCode, KeyModifiers, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, Signal, ValidationResult, Validator,
    default_emacs_keybindings,
};
use scraper::{Html, Selector};
use typsess::{
    ExecutionOutput, InputStatus, RenderMode, SourceMode, TypstReplSession, WorldOptions,
    classify_input,
};

use crate::output::{format_diagnostics, format_diagnostics_rich_with_sources};
use crate::session::create_session;

const REPL_PROMPT_LABEL: &str = "typst";

pub fn run(
    mode: RenderMode,
    source_mode: SourceMode,
    page_setup: String,
    full_html: bool,
    world_options: WorldOptions,
) -> Result<()> {
    let mut session = create_session(mode, source_mode, page_setup, world_options)
        .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?;
    let mut buffer = String::new();

    if io::stdin().is_terminal() {
        run_readline(&mut session, source_mode, full_html)
    } else {
        run_plain(&mut session, source_mode, &mut buffer, full_html)
    }
}

fn run_readline(
    session: &mut TypstReplSession,
    source_mode: SourceMode,
    full_html: bool,
) -> Result<()> {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(KeyModifiers::SHIFT, KeyCode::Enter, ReedlineEvent::Submit);
    let edit_mode = Box::new(Emacs::new(keybindings));
    let mut editor = Reedline::create()
        .with_edit_mode(edit_mode)
        .with_validator(Box::new(TypstValidator { source_mode }))
        .use_kitty_keyboard_enhancement(true);
    let prompt = TypstPrompt;
    let mut interrupted = false;

    loop {
        match editor.read_line(&prompt)? {
            Signal::Success(buffer) => {
                interrupted = false;
                execute_buffer(session, &buffer, full_html);
            }
            Signal::CtrlC => {
                if interrupted {
                    break;
                }
                interrupted = true;
            }
            Signal::CtrlD => break,
            Signal::ExternalBreak(buffer) => {
                interrupted = false;
                execute_buffer(session, &buffer, full_html);
            }
            _ => {}
        }
    }

    Ok(())
}

fn run_plain(
    session: &mut TypstReplSession,
    source_mode: SourceMode,
    buffer: &mut String,
    full_html: bool,
) -> Result<()> {
    print_prompt(false);
    let stdin = io::stdin();
    loop {
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        handle_plain_line(
            session,
            source_mode,
            buffer,
            line.trim_end_matches('\n'),
            full_html,
        );
        print_prompt(!buffer.is_empty());
    }
    Ok(())
}

fn handle_plain_line(
    session: &mut TypstReplSession,
    source_mode: SourceMode,
    buffer: &mut String,
    line: &str,
    full_html: bool,
) {
    buffer.push_str(line);
    buffer.push('\n');
    match classify_input(buffer, source_mode) {
        InputStatus::Complete => {
            execute_buffer(session, buffer, full_html);
            buffer.clear();
        }
        InputStatus::Incomplete(_) => {}
        InputStatus::Invalid(_) => {
            execute_buffer(session, buffer, full_html);
            buffer.clear();
        }
    }
}

fn execute_buffer(session: &mut TypstReplSession, buffer: &str, full_html: bool) {
    let result = match session.execute(buffer) {
        Ok(result) => result,
        Err(diagnostics) => {
            eprintln!(
                "{}",
                format_diagnostics_rich_with_sources(diagnostics, session.diagnostic_sources())
            );
            return;
        }
    };
    for warning in result.warnings {
        eprintln!("{}", warning.message);
    }
    let output = match execution_output_to_string(result.output, full_html) {
        Ok(output) => output,
        Err(diagnostics) => {
            eprintln!(
                "{}",
                format_diagnostics_rich_with_sources(diagnostics, session.diagnostic_sources())
            );
            return;
        }
    };
    println!("{output}");
}

pub(crate) fn execution_output_to_string(
    output: ExecutionOutput,
    full_html: bool,
) -> Result<String, ecow::EcoVec<typst::diag::SourceDiagnostic>> {
    match output {
        ExecutionOutput::Paged(document) => Ok(document
            .pages
            .iter()
            .map(typst_svg::svg)
            .collect::<Vec<_>>()
            .join("\n")),
        ExecutionOutput::Html(document) => {
            let html = typst_html::html(&document)?;
            if full_html {
                Ok(html)
            } else {
                Ok(body_inner_html(&html).unwrap_or(html))
            }
        }
    }
}

pub(crate) fn body_inner_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("body").ok()?;
    document
        .select(&selector)
        .next()
        .map(|body| body.inner_html().trim().to_string())
}

fn print_prompt(continuation: bool) {
    if continuation {
        print!("{}", continuation_prompt());
    } else {
        print!("{}", primary_prompt());
    }
    let _ = io::stdout().flush();
}

fn primary_prompt() -> String {
    format!("{REPL_PROMPT_LABEL}> ")
}

fn continuation_prompt() -> String {
    format!("{}> ", ".".repeat(REPL_PROMPT_LABEL.chars().count()))
}

struct TypstValidator {
    source_mode: SourceMode,
}

impl Validator for TypstValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        match classify_input(line, self.source_mode) {
            InputStatus::Incomplete(_) => ValidationResult::Incomplete,
            InputStatus::Complete | InputStatus::Invalid(_) => ValidationResult::Complete,
        }
    }
}

struct TypstPrompt;

impl Prompt for TypstPrompt {
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(primary_prompt())
    }

    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(continuation_prompt())
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        std::borrow::Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}
