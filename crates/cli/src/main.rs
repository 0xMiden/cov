//! The `miden-cov` command-line tool for collecting and reporting Miden VM code coverage.

mod report;
mod run;

use clap::{Parser, Subcommand};

/// Code coverage for Miden VM programs.
#[derive(Debug, Parser)]
#[command(name = "miden-cov", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The `miden-cov` subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Execute a program and collect its coverage.
    Run(Box<run::RunArgs>),
    /// Merge collected runs and render a coverage report.
    Report(Box<report::ReportArgs>),
    /// Print a run file as JSON.
    Dump(report::DumpArgs),
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Run(args) => run::run(*args),
        Command::Report(args) => report::report(*args),
        Command::Dump(args) => report::dump(args),
    }
}
