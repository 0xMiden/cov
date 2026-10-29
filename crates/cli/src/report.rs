//! The `report` and `dump` subcommands: read the coverage directory and
//! render it.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{ArgGroup, Args};
use miden_cov::{
    Report, ReportFilter, Snapshot, SourceResolver, build_report,
    compare::{PathMatcher, compare},
    diff::Diff,
    dump,
    html::{self, HtmlOptions},
    lcov,
    markdown::{self, MarkdownOptions},
    summary,
};
use miden_cov_collector::dir::CoverageDir;

/// Merge collected runs and render a coverage report.
#[derive(Debug, Args)]
#[command(group = ArgGroup::new("rendered").args(["html", "markdown"]).multiple(true))]
pub struct ReportArgs {
    /// The coverage directory to read.
    ///
    /// Defaults to `MIDEN_COV_DIR`, then to `target/miden-cov`.
    #[arg(long, value_name = "DIR")]
    cov_dir: Option<PathBuf>,
    /// Write an LCOV trace file to this path.
    #[arg(long, value_name = "FILE")]
    lcov: Option<PathBuf>,
    /// Write a self-contained HTML page with every file's source and its
    /// coverage to this path.
    #[arg(long, value_name = "FILE")]
    html: Option<PathBuf>,
    /// Write a pull-request comment in markdown to this path.
    #[arg(long, value_name = "FILE")]
    markdown: Option<PathBuf>,
    /// Compare with this LCOV trace file of the base branch in the page and
    /// the comment. Requires `--html` or `--markdown`.
    #[arg(long, value_name = "FILE", requires = "rendered")]
    baseline: Option<PathBuf>,
    /// Mark the lines this unified diff adds, for example the output of
    /// `git diff next...`, in the page and the comment. Its paths are
    /// relative to the source root. Requires `--html` or `--markdown`.
    #[arg(long, value_name = "FILE", requires = "rendered")]
    diff: Option<PathBuf>,
    /// Where the HTML page will be published, linked from the comment.
    /// Requires `--html` or `--markdown`.
    #[arg(long, value_name = "URL", requires = "rendered")]
    page_url: Option<String>,
    /// Heading of the page and the comment.
    #[arg(long, value_name = "TEXT", default_value = "Coverage")]
    title: String,
    /// Directory to resolve source paths against when they are not found
    /// as stored, for example a checkout of the library being measured.
    ///
    /// Only files under it, by default the current directory, are reported
    /// unless `--include` is given.
    #[arg(long, value_name = "DIR")]
    source_root: Option<PathBuf>,
    /// Replace a prefix of stored source paths before resolving them, as
    /// `OLD=NEW`. May be given several times; the first matching one wins.
    #[arg(long = "path-prefix-map", value_name = "OLD=NEW", value_parser = parse_prefix_map)]
    path_prefix_maps: Vec<(String, String)>,
    /// Report only source files matching this glob, wherever they are. May
    /// be given several times; a file is kept if it matches any. Relative
    /// globs such as `src/**` match paths relative to the source root.
    #[arg(long = "include", value_name = "GLOB")]
    includes: Vec<String>,
    /// Leave out source files matching this glob, even if included. May be
    /// given several times.
    #[arg(long = "exclude", value_name = "GLOB")]
    excludes: Vec<String>,
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
    let root = match &args.source_root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("cannot read the current directory")?,
    };
    let filter = ReportFilter::new(&args.includes, &args.excludes)?
        .relative_to(&root)
        .scoped_to(&root);
    let dir = match args.cov_dir {
        Some(dir) => CoverageDir::new(dir),
        None => CoverageDir::from_env_or_default(),
    };
    let snapshot = Snapshot::load(&dir)?;
    if snapshot.runs.is_empty() {
        bail!("no runs found in coverage directory `{}`", dir.root().display());
    }
    let paths = PathMatcher::new(&root, args.path_prefix_maps.clone());
    let mut resolver = SourceResolver::new(args.source_root, args.path_prefix_maps);
    let report = build_report(&snapshot, &mut resolver, &filter);

    if let Some(path) = &args.lcov {
        write_output(path, |out| lcov::write(&report, out))?;
    }
    if args.html.is_some() || args.markdown.is_some() {
        let base = args.baseline.as_deref().map(read_baseline).transpose()?;
        let diff = match &args.diff {
            Some(path) => Some(Diff::parse(&read_text(path, "diff")?)),
            None => None,
        };
        let comparison = compare(&report, base.as_ref(), diff.as_ref(), &paths);
        if let Some(path) = &args.html {
            let options = HtmlOptions {
                title: args.title.clone(),
                ..Default::default()
            };
            write_output(path, |out| html::write(&comparison, &options, out))?;
        }
        if let Some(path) = &args.markdown {
            let options = MarkdownOptions {
                title: args.title,
                page_url: args.page_url,
            };
            write_output(path, |out| markdown::write(&comparison, &options, out))?;
        }
    }
    if !args.no_summary {
        let mut out = io::stdout().lock();
        summary::write(&report, &mut out)?;
        for path in [&args.lcov, &args.html, &args.markdown].into_iter().flatten() {
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

/// Writes the output file `path` with `render`, creating its directory.
fn write_output(
    path: &Path,
    render: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create `{}`", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("cannot write `{}`", path.display()))?;
    let mut out = BufWriter::new(file);
    render(&mut out)
        .and_then(|()| out.flush())
        .with_context(|| format!("cannot write `{}`", path.display()))
}

/// Reads the text of the `what` file at `path`.
fn read_text(path: &Path, what: &str) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| format!("cannot read {what} `{}`", path.display()))
}

/// Reads the LCOV trace file of the baseline at `path`.
fn read_baseline(path: &Path) -> anyhow::Result<Report> {
    lcov::read(&read_text(path, "baseline")?)
        .with_context(|| format!("invalid baseline `{}`", path.display()))
}

fn parse_prefix_map(value: &str) -> Result<(String, String), String> {
    match value.split_once('=') {
        Some((old, new)) if !old.is_empty() => Ok((old.to_owned(), new.to_owned())),
        _ => Err(format!("expected OLD=NEW, got `{value}`")),
    }
}
