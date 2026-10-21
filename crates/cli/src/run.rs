//! The `run` subcommand: executes a package under the coverage collector and
//! writes one run file to the coverage directory.
//!
//! Package loading, library linking, and the inputs file mirror the
//! `miden-debug` command line, so a package that runs in the debugger runs
//! here with the same flags.

use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, anyhow, bail};
use clap::Args;
use miden_cov_collector::{Collector, RunOutcome, dir::CoverageDir};
use miden_debug_engine::{
    ExecutionConfig, Executor, HybridPackageRegistry, LinkLibrary,
    debug_types::{DefaultSourceManager, SourceManager, Uri},
    read_package_from_bytes,
};
use miden_mast_package::{Package, QualifiedProcedureName};

/// Execute a compiled Miden package and record its coverage.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Path to the Miden package (`.masp`) to execute.
    ///
    /// The package must carry debug info.
    #[arg(value_name = "FILE")]
    package: PathBuf,
    /// Path to a TOML file with the program inputs (operand stack, advice
    /// stack, advice map, execution options), in the `miden-debug` format.
    #[arg(long, value_name = "FILE")]
    inputs: Option<ExecutionConfig>,
    /// The procedure to use as the entrypoint of a library package, in the
    /// format `<module_name>::<function>`.
    #[arg(long, help_heading = "Execution")]
    entrypoint: Option<String>,
    /// The path to the root directory of the current Miden toolchain; its
    /// `lib` directory provides the system libraries.
    #[arg(long, value_name = "DIR", env = "MIDEN_SYSROOT", help_heading = "Linker")]
    sysroot: Option<PathBuf>,
    /// Specify one or more search paths for link libraries requested via `-l`.
    #[arg(long = "search-path", short = 'L', value_name = "PATH", help_heading = "Linker")]
    search_path: Vec<PathBuf>,
    /// Load the compiled library package NAME.
    ///
    /// NAME must either be a path (with extension), or a package namespace (no
    /// extension). The former is loaded from that path, the latter is located
    /// in the library search paths.
    ///
    /// KIND currently supports only `masp` (the default). The optional LINKAGE
    /// is either `static` or `dynamic` and defaults to `dynamic`.
    #[arg(
        long = "link-library",
        short = 'l',
        value_name = "[KIND[:LINKAGE]=]NAME",
        value_delimiter = ',',
        next_line_help(true),
        help_heading = "Linker"
    )]
    link_libraries: Vec<LinkLibrary>,
    /// The coverage directory to write to.
    ///
    /// Defaults to the `MIDEN_COV_DIR` environment variable, else
    /// `target/miden-cov`.
    #[arg(long, value_name = "DIR", help_heading = "Coverage")]
    cov_dir: Option<PathBuf>,
    /// Name recorded for this run. Defaults to the package file stem.
    #[arg(long, help_heading = "Coverage")]
    name: Option<String>,
}

/// Executes the package described by `args`, writes its coverage, and prints
/// the run file path.
///
/// A failing program still produces a run file; the execution error is
/// returned only after the file is written.
pub fn run(args: RunArgs) -> anyhow::Result<()> {
    let package = load_package(&args.package, args.entrypoint.as_deref())?;
    if package.debug_info().context("invalid debug info")?.is_none() {
        bail!(
            "package '{}' has no debug info, so its coverage cannot be attributed to source; set \
             `debug = true` under `[profile.<name>]` in `miden-project.toml` and rebuild (the \
             release profile strips debug info, the dev profile keeps it)",
            args.package.display()
        );
    }

    let registry = HybridPackageRegistry::new(
        args.sysroot.as_deref(),
        &args.search_path,
        &args.link_libraries,
    )
    .map_err(from_report)
    .context("failed to load link libraries")?;
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let (mut stepper, mut host) = Executor::from_config(args.inputs.unwrap_or_default())
        .with_registry(registry)
        .into_stepper(package, source_manager);

    // The engine describes each cycle to the collector before executing it.
    let mut collector = Collector::new();
    let result = stepper.run_sync(&mut host, |cycle| collector.on_cycle(cycle));

    let name = args.name.unwrap_or_else(|| default_run_name(&args.package));
    let output = collector.finish(name, RunOutcome::of(&result));
    let cov_dir = args.cov_dir.map_or_else(CoverageDir::from_env_or_default, CoverageDir::new);
    let path = cov_dir
        .write_output(&output)
        .with_context(|| format!("failed to write coverage to '{}'", cov_dir.root().display()))?;
    println!("wrote {} ({} cycles)", path.display(), output.run.cycles);

    result.context("program execution failed")
}

/// Reads the package at `path` and makes it executable.
///
/// A library package needs `entrypoint`, which selects the exported procedure
/// to run; a program package must not have one.
fn load_package(path: &Path, entrypoint: Option<&str>) -> anyhow::Result<Arc<Package>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read package '{}'", path.display()))?;
    let package = read_package_from_bytes(&bytes, &Uri::from(path)).map_err(from_report)?;

    match entrypoint {
        Some(entrypoint) => {
            let id = entrypoint
                .parse::<QualifiedProcedureName>()
                .map_err(|_| anyhow!("invalid function identifier: '{entrypoint}'"))?;
            if !package.is_library() {
                bail!("cannot use --entrypoint with executable packages");
            }
            package.make_executable(&id).map(Arc::new).map_err(from_report)
        },
        None if package.is_program() => Ok(package),
        None => bail!(
            "input package '{}' is not executable; pass --entrypoint <module>::<procedure> to run \
             a library package",
            path.display()
        ),
    }
}

/// The run name used when none is given: the package file stem.
fn default_run_name(package: &Path) -> String {
    package
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Converts a diagnostic report from the Miden crates into an `anyhow` error,
/// keeping its cause chain.
fn from_report(report: impl Into<Box<dyn Error + Send + Sync>>) -> anyhow::Error {
    anyhow::Error::from_boxed(report.into())
}
