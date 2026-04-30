use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Result, anyhow};
use reedline::{
    Emacs, KeyCode, KeyModifiers, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, Signal, ValidationResult, Validator,
    default_emacs_keybindings,
};
use typsess::{InputStatus, PageSetup, RenderMode, TypstReplSession, classify_input};

use crate::output::{execution_output_to_cli_html, format_diagnostics, format_diagnostics_rich};

const REPL_PROMPT_LABEL: &str = "typst";

pub fn run(mode: RenderMode, page_setup: PageSetup, full_html: bool) -> Result<()> {
    let mut session = TypstReplSession::new(mode, page_setup)
        .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?;
    let mut buffer = String::new();

    if io::stdin().is_terminal() {
        run_readline(&mut session, full_html)
    } else {
        run_plain(&mut session, &mut buffer, full_html)
    }
}

fn run_readline(session: &mut TypstReplSession, full_html: bool) -> Result<()> {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(KeyModifiers::SHIFT, KeyCode::Enter, ReedlineEvent::Submit);
    let edit_mode = Box::new(Emacs::new(keybindings));
    let mut editor = Reedline::create()
        .with_edit_mode(edit_mode)
        .with_validator(Box::new(TypstValidator))
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

fn run_plain(session: &mut TypstReplSession, buffer: &mut String, full_html: bool) -> Result<()> {
    print_prompt(false);
    let stdin = io::stdin();
    loop {
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        handle_plain_line(session, buffer, line.trim_end_matches('\n'), full_html);
        print_prompt(!buffer.is_empty());
    }
    Ok(())
}

fn handle_plain_line(
    session: &mut TypstReplSession,
    buffer: &mut String,
    line: &str,
    full_html: bool,
) {
    buffer.push_str(line);
    buffer.push('\n');
    match classify_input(buffer) {
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
            eprintln!("{}", format_diagnostics_rich(diagnostics, buffer));
            return;
        }
    };
    for warning in result.warnings {
        eprintln!("{}", warning.message);
    }
    let html = match execution_output_to_cli_html(result.output, full_html) {
        Ok(html) => html,
        Err(diagnostics) => {
            eprintln!("{}", format_diagnostics_rich(diagnostics, buffer));
            return;
        }
    };
    println!("{html}");
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

struct TypstValidator;

impl Validator for TypstValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        match classify_input(line) {
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
