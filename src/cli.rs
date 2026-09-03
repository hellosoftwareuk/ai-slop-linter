use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "slop",
    version,
    about = "Measure TypeScript, Rust, Terraform, and Terragrunt maintainability debt at native speed"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Folder, repository, or repository subfolder to scan.
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Apply only conservative, AST-proven TypeScript fixes, then rescan.
    #[arg(long)]
    pub fix: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Maximum number of findings shown in text output.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Exit with code 2 when the slop score is above this value.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub fail_above: Option<u8>,

    /// Include generated TypeScript declaration files (*.d.ts).
    #[arg(long)]
    pub include_declarations: bool,

    /// Scan hidden and gitignored files.
    #[arg(long)]
    pub no_ignore: bool,

    /// Worker threads. Zero lets the walker choose.
    #[arg(long, default_value_t = 0)]
    pub threads: usize,

    /// Skip individual files larger than this many bytes.
    #[arg(long, default_value_t = 2_000_000)]
    pub max_file_bytes: u64,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install or run the zero-configuration Codex integration.
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CodexCommand {
    /// Register Slop hooks in the nearest repository.
    Install {
        /// Repository or subfolder in which to install the hooks.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Handle one Codex hook event from standard input.
    #[command(hide = true)]
    Hook,
}
