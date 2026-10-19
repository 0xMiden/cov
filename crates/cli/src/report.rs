//! The `report` and `dump` subcommands: read the coverage directory and
//! render it.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::Args;
use miden_cov::{Snapshot, SourceResolver, build_report, dump, lcov, summary};
use miden_cov_collector::dir::CoverageDir;

/// Merge collected runs and render a coverage report.
#[derive(Debug, Args)]
pub struct ReportArgs {
    /// The coverage directory to read.
    ///
    /// Defaults to `MIDEN_COV_DIR`, then to `target/miden-cov`.
    #[arg(long, value_name = "DIR")]
    cov_dir: Option<PathBuf>,
    /// Write an LCOV trace file to this path.
    #[arg(long, value_name = "FILE")]
    lcov: Option<PathBuf>,
    /// Directory to resolve source paths against when they are not found
    /// as stored, for example a checkout of the library being measured.
    #[arg(long, value_name = "DIR")]
    source_root: Option<PathBuf>,
    /// Replace a prefix of stored source paths before resolving them, as
    /// `OLD=NEW`. May be given several times; the first matching one wins.
    #[arg(long = "path-prefix-map", value_name = "OLD=NEW", value_parser = parse_prefix_map)]
    path_prefix_maps: Vec<(String, String)>,
    /// Do not print the summary table.
    #[arg(long)]
    no_summary: bool,
}

/// Print a run file as JSON.
#[derive(Debug, Args)]
pub struct DumpArgs {
    /// The run file to print.
    #[arg(value_name = "FILE")]
    run: PathBuf,
}

/// Runs the `report` subcommand.
pub fn report(args: ReportArgs) -> anyhow::Result<()> {
    let dir = match args.cov_dir {
        Some(dir) => CoverageDir::new(dir),
        None => CoverageDir::from_env_or_default(),
    };
    let snapshot = Snapshot::load(&dir)?;
    if snapshot.runs.is_empty() {
        bail!("no runs found in coverage directory `{}`", dir.root().display());
    }
    let mut resolver = SourceResolver::new(args.source_root, args.path_prefix_maps);
    let report = build_report(&snapshot, &mut resolver);

    if let Some(path) = &args.lcov {
        write_lcov(&report, path)?;
    }
    if !args.no_summary {
        let mut out = io::stdout().lock();
        summary::write(&report, &mut out)?;
        if let Some(path) = &args.lcov {
            writeln!(out, "wrote {}", path.display())?;
        }
    }
    Ok(())
}

/// Runs the `dump` subcommand.
pub fn dump(args: DumpArgs) -> anyhow::Result<()> {
    let run = CoverageDir::read_run(&args.run)
        .with_context(|| format!("cannot read run file `{}`", args.run.display()))?;
    let json = dump::run_to_json(&run);
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &json)?;
    writeln!(out)?;
    Ok(())
}

fn write_lcov(report: &miden_cov::Report, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create `{}`", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("cannot write `{}`", path.display()))?;
    let mut out = BufWriter::new(file);
    lcov::write(report, &mut out)?;
    out.flush()?;
    Ok(())
}

fn parse_prefix_map(value: &str) -> Result<(String, String), String> {
    match value.split_once('=') {
        Some((old, new)) if !old.is_empty() => Ok((old.to_owned(), new.to_owned())),
        _ => Err(format!("expected OLD=NEW, got `{value}`")),
    }
}
