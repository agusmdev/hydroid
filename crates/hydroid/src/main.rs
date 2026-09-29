use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use hydroid::{Options, check, render};

/// Finds blocking calls reachable on the event loop in async FastAPI code.
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
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = Options {
        root: cli.path,
        python: cli.python,
        follow_libs: cli.follow_libs,
        cpu: cli.cpu,
        catalogs: Vec::new(),
    };
    let report = match check(&options) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("hydroid: {err:#}");
            return ExitCode::from(2);
        }
    };
    match cli.format {
        Format::Human => print!("{}", render::human(&report, cli.strict)),
        Format::Json => println!("{}", serde_json::to_string_pretty(&report).expect("reports serialize")),
    }
    let failed = !report.diagnostics.is_empty() || (cli.strict && !report.unresolved.is_empty());
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}
