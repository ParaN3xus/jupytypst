use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use jupyter_protocol::JupyterKernelspec;
use tempfile::TempDir;
use typsess::{PageSetup, RenderMode, SourceMode, WorldOptions};

mod cell;
mod kernel;
mod output;
mod repl;

const CODE_KERNEL_NAME: &str = "jupytypst-code";
const MARKUP_KERNEL_NAME: &str = "jupytypst-markup";
const CODE_DISPLAY_NAME: &str = "Typst (Code Mode)";
const MARKUP_DISPLAY_NAME: &str = "Typst";
const ENV_PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

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
    /// Start an interactive Typst REPL.
    Repl(ReplArgs),
    /// Install the Jupyter kernelspec for this binary.
    Install(InstallArgs),
}

#[derive(Debug, Args)]
struct StartArgs {
    /// Path to the Jupyter connection file.
    #[arg(long = "connection-file")]
    connection_file: PathBuf,
    /// Page setup injected before each rendered cell. Omit for `set page(width: auto, height: auto, margin: 16pt)`, use `none` to disable, or pass Typst code.
    #[arg(long)]
    page_setup: Option<String>,
    /// The format of rendered kernel output.
    #[arg(short = 'f', long = "format", value_enum, default_value_t = CliOutputFormat::Svg)]
    format: CliOutputFormat,
    /// Source parsing mode for executed cells.
    #[arg(long, value_enum, default_value_t = CliSourceMode::Code)]
    mode: CliSourceMode,
    #[command(flatten)]
    world: WorldArgs,
}

#[derive(Debug, Args)]
struct ReplArgs {
    /// The format of rendered terminal output.
    #[arg(short = 'f', long = "format", value_enum, default_value_t = CliOutputFormat::Html)]
    format: CliOutputFormat,
    /// Source parsing mode for REPL input.
    #[arg(long, value_enum, default_value_t = CliSourceMode::Code)]
    mode: CliSourceMode,
    /// Print complete HTML documents instead of only the body contents.
    #[arg(long)]
    full_html: bool,
    /// Page setup for rendered cells. Omit for `set page(width: auto, height: auto, margin: 16pt)`, use `none` to disable, or pass Typst code.
    #[arg(long)]
    page_setup: Option<String>,
    #[command(flatten)]
    world: WorldArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliOutputFormat {
    Pdf,
    Png,
    Svg,
    Html,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSourceMode {
    Code,
    Markup,
}

impl From<CliSourceMode> for SourceMode {
    fn from(value: CliSourceMode) -> Self {
        match value {
            CliSourceMode::Code => Self::Code,
            CliSourceMode::Markup => Self::Markup,
        }
    }
}

impl CliSourceMode {
    fn kernel_name(self) -> &'static str {
        match self {
            Self::Code => CODE_KERNEL_NAME,
            Self::Markup => MARKUP_KERNEL_NAME,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Code => CODE_DISPLAY_NAME,
            Self::Markup => MARKUP_DISPLAY_NAME,
        }
    }

    fn language(self) -> &'static str {
        match self {
            Self::Code => "typst-code",
            Self::Markup => "typst",
        }
    }
}

impl TryFrom<CliOutputFormat> for RenderMode {
    type Error = anyhow::Error;

    fn try_from(value: CliOutputFormat) -> Result<Self> {
        match value {
            CliOutputFormat::Svg => Ok(Self::Svg),
            CliOutputFormat::Html => Ok(Self::Html),
            CliOutputFormat::Pdf | CliOutputFormat::Png => {
                bail!("format `{}` is not supported yet", value.as_str())
            }
        }
    }
}

impl CliOutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Html => "html",
        }
    }
}

#[derive(Debug, Args)]
struct WorldArgs {
    /// Configures the project root (for absolute paths).
    #[arg(long, value_name = "DIR", env = "TYPST_ROOT")]
    root: Option<PathBuf>,
    /// Add a string key-value pair visible through `sys.inputs`.
    #[arg(long = "input", value_name = "key=value", value_parser = parse_input_pair)]
    inputs: Vec<(String, String)>,
    /// Adds additional directories that are recursively searched for fonts.
    #[arg(
        long = "font-path",
        value_name = "DIR",
        env = "TYPST_FONT_PATHS",
        value_delimiter = ENV_PATH_SEP
    )]
    font_paths: Vec<PathBuf>,
    /// Ensures system fonts won't be searched, unless explicitly included via `--font-path`.
    #[arg(long, env = "TYPST_IGNORE_SYSTEM_FONTS")]
    ignore_system_fonts: bool,
    /// Ensures fonts embedded into Typst won't be considered.
    #[arg(long, env = "TYPST_IGNORE_EMBEDDED_FONTS")]
    ignore_embedded_fonts: bool,
    /// Custom path to local packages, defaults to system-dependent location.
    #[arg(long, value_name = "DIR", env = "TYPST_PACKAGE_PATH")]
    package_path: Option<PathBuf>,
    /// Custom path to package cache, defaults to system-dependent location.
    #[arg(long, value_name = "DIR", env = "TYPST_PACKAGE_CACHE_PATH")]
    package_cache_path: Option<PathBuf>,
}

impl From<WorldArgs> for WorldOptions {
    fn from(value: WorldArgs) -> Self {
        Self {
            root: value.root,
            inputs: value.inputs,
            font_paths: value.font_paths,
            ignore_system_fonts: value.ignore_system_fonts,
            ignore_embedded_fonts: value.ignore_embedded_fonts,
            package_path: value.package_path,
            package_cache_path: value.package_cache_path,
        }
    }
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
        CommandKind::Repl(args) => start_repl(args),
        CommandKind::Install(args) => install_kernelspec(args),
    }
}

async fn start_kernel(args: StartArgs) -> Result<()> {
    kernel::run(
        args.connection_file,
        args.page_setup.unwrap_or_else(|| "default".to_string()),
        args.format.try_into()?,
        args.mode.into(),
        args.world.into(),
    )
    .await
}

fn start_repl(args: ReplArgs) -> Result<()> {
    let page_setup = parse_page_setup(args.page_setup)?;
    repl::run(
        args.format.try_into()?,
        args.mode.into(),
        page_setup,
        args.full_html,
        args.world.into(),
    )
}

fn parse_page_setup(page_setup: Option<String>) -> Result<PageSetup> {
    PageSetup::parse(page_setup.as_deref().unwrap_or("default"))
}

fn parse_input_pair(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| "input must be a key and a value separated by an equal sign".to_string())?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("the key was missing or empty".to_string());
    }
    Ok((key, value.trim().to_string()))
}

fn install_kernelspec(args: InstallArgs) -> Result<()> {
    let binary = std::env::current_exe().context("failed to locate current executable")?;
    let temp_dir = TempDir::new().context("failed to create temporary kernelspec directory")?;
    for mode in [CliSourceMode::Code, CliSourceMode::Markup] {
        let spec_dir = temp_dir.path().join(mode.kernel_name());
        fs::create_dir(&spec_dir).context("failed to create kernelspec staging directory")?;
        write_kernel_json(&spec_dir, &binary, mode)?;
        install_one_kernelspec(&args, &spec_dir, mode.kernel_name())?;
    }

    Ok(())
}

fn install_one_kernelspec(args: &InstallArgs, spec_dir: &Path, name: &str) -> Result<()> {
    let mut command = Command::new(&args.jupyter);
    command.arg("kernelspec").arg("install").arg(spec_dir);
    command.arg("--name").arg(name);
    if args.user {
        command.arg("--user");
    }
    if args.sys_prefix {
        command.arg("--sys-prefix");
    }
    if let Some(prefix) = &args.prefix {
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

fn write_kernel_json(spec_dir: &Path, binary: &Path, mode: CliSourceMode) -> Result<()> {
    let kernelspec = JupyterKernelspec {
        argv: vec![
            binary.display().to_string(),
            "start".to_string(),
            "--connection-file".to_string(),
            "{connection_file}".to_string(),
            "--mode".to_string(),
            mode.to_possible_value()
                .expect("source mode should have a clap value")
                .get_name()
                .to_string(),
        ],
        display_name: mode.display_name().to_string(),
        language: mode.language().to_string(),
        metadata: Some(HashMap::new()),
        interrupt_mode: Some("message".to_string()),
        env: Some(HashMap::new()),
    };

    let mut json = serde_json::to_value(&kernelspec)?;
    json["kernel_protocol_version"] = kernel::JUPYTER_PROTOCOL_VERSION.into();
    let json = serde_json::to_string_pretty(&json)?;
    fs::write(spec_dir.join("kernel.json"), json).context("failed to write kernel.json")?;
    Ok(())
}
