use std::{process::ExitCode, time::Instant};

use anyhow::Result;
use clap::Parser;
use slop::{
    cli::{Cli, CodexCommand, Command, DiffOutputFormat, OutputFormat},
    codex, diff,
    discovery::{scan, ScanOptions},
    fixer,
    report::{print_json, print_text},
    scoring::build_report,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            eprintln!(
                "LLM remediation prompt: {}",
                slop::remediation::fatal_prompt(&error)
            );
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return match command {
            Command::Diff(args) => {
                let report = diff::analyze(&args.path, args.base.as_deref(), args.staged)?;
                match args.format {
                    DiffOutputFormat::Text => diff::print_text(&report, args.top),
                    DiffOutputFormat::Json => diff::print_json(&report)?,
                    DiffOutputFormat::Github => diff::print_github(&report),
                }
                Ok(if report.has_new_debt() {
                    ExitCode::from(2)
                } else {
                    ExitCode::SUCCESS
                })
            }
            Command::Codex { command } => match command {
                CodexCommand::Install { path } => {
                    let installed = codex::install(&path)?;
                    println!(
                        concat!(
                            "Installed Slop Codex hooks in {}. ",
                            "Restart Codex and approve the repository hooks when prompted."
                        ),
                        installed.display()
                    );
                    Ok(ExitCode::SUCCESS)
                }
                CodexCommand::Hook => {
                    codex::run_hook()?;
                    Ok(ExitCode::SUCCESS)
                }
            },
        };
    }
    let root = cli
        .path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("cannot access '{}': {error}", cli.path.display()))?;

    let started = Instant::now();
    let options = ScanOptions {
        include_declarations: cli.include_declarations,
        respect_ignores: !cli.no_ignore,
        max_file_bytes: cli.max_file_bytes,
        threads: cli.threads,
    };
    let mut analyses = scan(&root, &options)?;
    let fixes = if cli.fix {
        let summary = fixer::apply(&root, &analyses)?;
        analyses = scan(&root, &options)?;
        summary
    } else {
        Default::default()
    };
    let mut report = build_report(&root, analyses, started.elapsed());
    report.fixes = fixes;

    match cli.format {
        OutputFormat::Text => print_text(&report, cli.top),
        OutputFormat::Json => print_json(&report)?,
    }

    if cli
        .fail_above
        .is_some_and(|threshold| report.score > threshold)
    {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}
