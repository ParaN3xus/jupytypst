use std::io::{self, Write};

use anyhow::Result;
use typst_repl::{
    ExecutionOutput, InputStatus, PageSetup, RenderMode, TypstReplSession, classify_input,
};

pub fn run(mode: RenderMode, page_setup: PageSetup) -> Result<()> {
    let mut session = TypstReplSession::new(mode, page_setup)?;
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
                execute_buffer(&mut session, &buffer)?;
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
                execute_buffer(session, buffer)?;
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

fn execute_buffer(session: &mut TypstReplSession, buffer: &str) -> Result<()> {
    let result = session.execute(buffer)?;
    for warning in result.warnings {
        eprintln!("{warning}");
    }
    match result.output {
        ExecutionOutput::Html(html) | ExecutionOutput::Svg(html) => {
            println!("{html}");
        }
    }
    Ok(())
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
