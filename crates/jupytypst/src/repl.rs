use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Result, anyhow};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use typsess::{InputStatus, PageSetup, RenderMode, TypstReplSession, classify_input};

use crate::output::{execution_output_to_cli_html, format_diagnostics, format_diagnostics_rich};

pub fn run(mode: RenderMode, page_setup: PageSetup, full_html: bool) -> Result<()> {
    let mut session = TypstReplSession::new(mode, page_setup)
        .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?;
    let mut buffer = String::new();

    if io::stdin().is_terminal() {
        run_readline(&mut session, &mut buffer, full_html)
    } else {
        run_plain(&mut session, &mut buffer, full_html)
    }
}

fn run_readline(
    session: &mut TypstReplSession,
    buffer: &mut String,
    full_html: bool,
) -> Result<()> {
    let mut editor = DefaultEditor::new()?;
    loop {
        let prompt = if buffer.is_empty() {
            "typst> "
        } else {
            "....> "
        };
        match editor.readline(prompt) {
            Ok(line) => {
                let _ = editor.add_history_entry(line.as_str());
                if handle_line(session, buffer, &line, full_html)? {
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => {
                buffer.clear();
            }
            Err(ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
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
        if handle_line(session, buffer, line.trim_end_matches('\n'), full_html)? {
            break;
        }
        print_prompt(!buffer.is_empty());
    }
    Ok(())
}

fn handle_line(
    session: &mut TypstReplSession,
    buffer: &mut String,
    line: &str,
    full_html: bool,
) -> Result<bool> {
    if line.starts_with('.') {
        return handle_command(line, buffer, session, full_html);
    }

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
    Ok(false)
}

fn handle_command(
    command: &str,
    buffer: &mut String,
    session: &mut TypstReplSession,
    full_html: bool,
) -> Result<bool> {
    match command {
        ".exit" | ".quit" => Ok(true),
        ".clear" => {
            buffer.clear();
            Ok(false)
        }
        ".help" => {
            eprintln!(".exit/.quit  exit the REPL");
            eprintln!(".clear       clear the current input buffer");
            eprintln!(".run         execute the current input buffer");
            eprintln!(".help        show this help");
            Ok(false)
        }
        ".run" => {
            if !buffer.trim().is_empty() {
                execute_buffer(session, buffer, full_html);
                buffer.clear();
            }
            Ok(false)
        }
        other => {
            eprintln!("unknown command `{other}`");
            Ok(false)
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
        print!("....> ");
    } else {
        print!("typst> ");
    }
    let _ = io::stdout().flush();
}
