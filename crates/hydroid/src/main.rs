use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use hydroid::config::Config;
use hydroid::{Options, check, render};

/// Finds blocking calls reachable on the event loop in async FastAPI code.
///
/// Options also read from `[tool.hydroid]` in `<PATH>/pyproject.toml`; flags take precedence.
/// Exit status: 0 clean, 1 blocking calls found (or unresolved calls with --strict), 2 error.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Project root.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Virtual environment or interpreter to resolve imports against (default: discovered).
    #[arg(long)]
    python: Option<PathBuf>,
    /// Analyze the bodies of third-party functions too.
    #[arg(long)]
    follow_libs: bool,
    /// Also report CPU-bound calls (password hashing, key derivation).
    #[arg(long)]
    cpu: bool,
    /// Report calls on the event loop that could not be resolved, and fail on them.
    #[arg(long)]
    strict: bool,
    /// Gitignore-style glob of files to skip (repeatable).
    #[arg(long)]
    exclude: Vec<String>,
    /// Do not read or write the fact cache (`<PATH>/.hydroid_cache`).
    #[arg(long)]
    no_cache: bool,
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
    Sarif,
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let config = Config::load(&cli.path)?;
    let catalogs = config.catalog_document().into_iter().collect();
    let strict = cli.strict || config.strict;
    let options = Options {
        python: cli.python.or(config.python),
        follow_libs: cli.follow_libs || config.follow_libs,
        cpu: cli.cpu || config.cpu,
        catalogs,
        exclude: config.exclude.into_iter().chain(cli.exclude).collect(),
        cache: !cli.no_cache,
        root: cli.path,
    };
    let report = check(&options)?;
    match cli.format {
        Format::Human => print!("{}", render::human(&report, strict)),
        Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Format::Sarif => println!("{}", serde_json::to_string_pretty(&render::sarif(&report, strict))?),
    }
    let failed = !report.diagnostics.is_empty() || (strict && !report.unresolved.is_empty());
    Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn main() -> ExitCode {
    run(Cli::parse()).unwrap_or_else(|err| {
        eprintln!("hydroid: {err:#}");
        ExitCode::from(2)
    })
}
