use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use jupyter_protocol::JupyterKernelspec;
use tempfile::TempDir;

mod kernel;
mod typst_session;

const KERNEL_NAME: &str = "jupytypst";
const DISPLAY_NAME: &str = "Typst (jupytypst)";

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    /// Start the Jupyter kernel.
    Start(StartArgs),
    /// Install the Jupyter kernelspec for this binary.
    Install(InstallArgs),
}

#[derive(Debug, Args)]
struct StartArgs {
    /// Path to the Jupyter connection file.
    #[arg(short = 'f', long = "connection-file")]
    connection_file: PathBuf,
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Install into the current user's Jupyter data directory.
    #[arg(long, conflicts_with_all = ["sys_prefix", "prefix"])]
    user: bool,
    /// Install into the active Python environment.
    #[arg(long, conflicts_with_all = ["user", "prefix"])]
    sys_prefix: bool,
    /// Install into an explicit Jupyter prefix.
    #[arg(long, conflicts_with_all = ["user", "sys_prefix"])]
    prefix: Option<PathBuf>,
    /// Replace an existing kernelspec with the same name.
    #[arg(long)]
    replace: bool,
    /// Jupyter executable to invoke.
    #[arg(long, default_value = "jupyter")]
    jupyter: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        CommandKind::Start(args) => start_kernel(args).await,
        CommandKind::Install(args) => install_kernelspec(args),
    }
}

async fn start_kernel(args: StartArgs) -> Result<()> {
    kernel::run(args.connection_file).await
}

fn install_kernelspec(args: InstallArgs) -> Result<()> {
    let binary = std::env::current_exe().context("failed to locate current executable")?;
    let temp_dir = TempDir::new().context("failed to create temporary kernelspec directory")?;
    let spec_dir = temp_dir.path().join(KERNEL_NAME);
    fs::create_dir(&spec_dir).context("failed to create kernelspec staging directory")?;
    write_kernel_json(&spec_dir, &binary)?;

    let mut command = Command::new(&args.jupyter);
    command.arg("kernelspec").arg("install").arg(&spec_dir);
    command.arg("--name").arg(KERNEL_NAME);
    if args.user {
        command.arg("--user");
    }
    if args.sys_prefix {
        command.arg("--sys-prefix");
    }
    if let Some(prefix) = args.prefix {
        command.arg("--prefix").arg(prefix);
    }
    if args.replace {
        command.arg("--replace");
    }

    let status = command
        .status()
        .with_context(|| format!("failed to run `{}`", args.jupyter))?;
    if !status.success() {
        bail!("`{}` exited with status {}", args.jupyter, status);
    }

    Ok(())
}

fn write_kernel_json(spec_dir: &Path, binary: &Path) -> Result<()> {
    let kernelspec = JupyterKernelspec {
        argv: vec![
            binary.display().to_string(),
            "start".to_string(),
            "--connection-file".to_string(),
            "{connection_file}".to_string(),
        ],
        display_name: DISPLAY_NAME.to_string(),
        language: "typst".to_string(),
        metadata: Some(HashMap::new()),
        interrupt_mode: Some("message".to_string()),
        env: Some(HashMap::new()),
    };

    let json = serde_json::to_string_pretty(&kernelspec)?;
    fs::write(spec_dir.join("kernel.json"), json).context("failed to write kernel.json")?;
    Ok(())
}
