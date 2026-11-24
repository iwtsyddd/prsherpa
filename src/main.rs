mod config;
mod git;
mod report;
mod rules;
mod score;

use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Terminal,
    Json,
    Markdown,
}

#[derive(Debug, Parser)]
#[command(
    name = "prsherpa",
    version,
    about = "Analyze a git branch before opening a pull request"
)]
struct Cli {
    /// Base branch to compare against (default: origin/main, origin/master, main, master)
    #[arg(long)]
    base: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Terminal)]
    format: OutputFormat,

    /// Backwards-compatible alias for --format json
    #[arg(long, hide = true)]
    json: bool,

    /// Exit with code 1 if the reviewability score is below this threshold (0-100)
    #[arg(long, value_name = "N")]
    fail_under: Option<u8>,

    /// Path to .prsherpa.toml config file
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

fn run(cli: Cli) -> Result<bool> {
    if let Some(threshold) = cli.fail_under {
        if threshold > 100 {
            bail!("--fail-under must be between 0 and 100 (got {threshold}).");
        }
    }

    let repo_root = git::repo_root(None)?;
    let cfg = config::load_config(cli.config.as_deref(), Some(&repo_root))?;
    let effective_base = cli.base.as_deref().or(cfg.base_branch.as_deref());

    let (base_branch, diff_files) = git::collect_diff(effective_base, None)?;
    let analysis = rules::analyze(&diff_files, &cfg);
    if analysis.files.is_empty() {
        bail!(
            "No changes found compared to '{base_branch}'. Make some changes or specify a different --base branch."
        );
    }

    let score = score::compute_score(&analysis.issues);
    let report = report::Report::from_analysis(base_branch, score, analysis);

    let format = if cli.json {
        OutputFormat::Json
    } else {
        cli.format
    };

    match format {
        OutputFormat::Json => println!("{}", report::render_json(&report)?),
        OutputFormat::Markdown => print!("{}", report::render_markdown(&report)),
        OutputFormat::Terminal => {
            let use_color =
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
            print!("{}", report::render_terminal(&report, use_color));
        }
    }

    let failed_threshold = cli.fail_under.is_some_and(|min| score < min);
    Ok(failed_threshold)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(true) => ExitCode::from(1),
        Ok(false) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::from(2)
        }
    }
}
