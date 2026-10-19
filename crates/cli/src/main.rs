//! The `miden-cov` command-line tool for collecting and reporting Miden VM code coverage.

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
    /// Render a coverage report from collected coverage data.
    Report,
    /// Dump collected coverage data in a human-readable form.
    Dump,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Run(args) => run::run(*args),
        Command::Report | Command::Dump => anyhow::bail!("not implemented"),
    }
}
