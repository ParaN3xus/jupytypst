use std::io::{self, Write};

use anyhow::{Result, anyhow};
use typsess::{InputStatus, PageSetup, RenderMode, TypstReplSession, classify_input};

use crate::output::{execution_output_to_html, format_diagnostics};

pub fn run(mode: RenderMode, page_setup: PageSetup) -> Result<()> {
    let mut session = TypstReplSession::new(mode, page_setup)
        .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?;
    let mut buffer = String::new();
    let stdin = io::stdin();

    print_prompt(false)?;
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }

        let command = line.trim_end();
        if command.starts_with('.') {
            if handle_command(command, &mut buffer, &mut session)? {
                break;
            }
            print_prompt(!buffer.is_empty())?;
            continue;
        }

        buffer.push_str(&line);
        match classify_input(&buffer) {
            InputStatus::Complete => {
                execute_buffer(&mut session, &buffer);
                buffer.clear();
                print_prompt(false)?;
            }
            InputStatus::Incomplete(_) => {
                print_prompt(true)?;
            }
            InputStatus::Invalid(error) => {
                eprintln!("{error}");
                buffer.clear();
                print_prompt(false)?;
            }
        }
    }

    Ok(())
}

fn handle_command(
    command: &str,
    buffer: &mut String,
    session: &mut TypstReplSession,
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
                execute_buffer(session, buffer);
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

fn execute_buffer(session: &mut TypstReplSession, buffer: &str) {
    let result = match session.execute(buffer) {
        Ok(result) => result,
        Err(diagnostics) => {
            eprintln!("{}", format_diagnostics(diagnostics));
            return;
        }
    };
    for warning in result.warnings {
        eprintln!("{}", warning.message);
    }
    let html = match execution_output_to_html(result.output) {
        Ok(html) => html,
        Err(diagnostics) => {
            eprintln!("{}", format_diagnostics(diagnostics));
            return;
        }
    };
    println!("{html}");
}

fn print_prompt(continuation: bool) -> Result<()> {
    if continuation {
        print!("....> ");
    } else {
        print!("typst> ");
    }
    io::stdout().flush()?;
    Ok(())
}
