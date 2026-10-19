//! Drives the `miden-cov run` binary on an assembled fixture and checks the
//! coverage directory it writes.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use miden_assembly::Assembler;
use miden_cov_collector::{RunOutcome, dir::CoverageDir};
use miden_mast_package::{Package, Word};

/// Assembles the fixture from its path, so debug locations carry the real
/// file path, and writes it to `dir`. Returns the package and its file.
fn assemble_fixture(dir: &Path) -> (Package, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fixture.masm");
    let package = *Assembler::default().assemble_program("fixture", source).unwrap();
    let path = dir.join("fixture.masp");
    package.write_to_file(&path).unwrap();
    (package, path)
}

/// Writes an inputs file with `stack` as the operand stack, top first.
fn write_inputs(dir: &Path, name: &str, stack: [u64; 2]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("[inputs]\nstack = [{}, {}]\n", stack[0], stack[1])).unwrap();
    path
}

/// A `miden-cov run` invocation of `package` writing to the coverage
/// directory `cov`.
fn miden_cov_run(package: &Path, cov: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_miden-cov"));
    command.arg("run").arg(package).arg("--cov-dir").arg(cov);
    command
}

/// Number of entries in `dir`.
fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir).unwrap().count()
}

/// Checks that a successful invocation reported the run file it wrote.
fn assert_wrote(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.starts_with("wrote "), "unexpected stdout: {stdout}");
}

/// Checks a run's name, outcome, and that it holds hits for the fixture only.
fn assert_run(run: &miden_cov_collector::RunFile, name: &str, commitment: Word) {
    assert_eq!(run.name, name);
    assert!(run.cycles > 0);
    assert_eq!(run.packages.len(), 1);
    assert_eq!(run.packages[0].0, commitment);
}

#[test]
fn runs_are_written_to_the_coverage_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let (package, masp) = assemble_fixture(tmp.path());
    let commitment = package.mast_forest_commitment();
    let cov = tmp.path().join("cov");
    let taken = write_inputs(tmp.path(), "taken.toml", [1, 3]);
    let skipped = write_inputs(tmp.path(), "skipped.toml", [0, 0]);

    let output = miden_cov_run(&masp, &cov)
        .arg("--inputs")
        .arg(&taken)
        .args(["--name", "taken"])
        .output()
        .unwrap();
    assert_wrote(&output);
    let output = miden_cov_run(&masp, &cov).arg("--inputs").arg(&skipped).output().unwrap();
    assert_wrote(&output);

    assert_eq!(count_files(&cov.join("packages")), 1);
    assert_eq!(count_files(&cov.join("runs")), 2);
    let runs = CoverageDir::new(&cov).read_runs().unwrap();
    assert_eq!(runs.len(), 2);
    // Run files are named by creation time, so they read back in run order.
    assert_run(&runs[0], "taken", commitment);
    assert_eq!(runs[0].outcome, RunOutcome::Ok);
    assert_run(&runs[1], "fixture", commitment);
    assert_eq!(runs[1].outcome, RunOutcome::Ok);

    // A non-binary condition fails the program, but the run is still written.
    let invalid = write_inputs(tmp.path(), "invalid.toml", [3, 0]);
    let output = miden_cov_run(&masp, &cov)
        .arg("--inputs")
        .arg(&invalid)
        .args(["--name", "invalid"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("wrote "));
    assert_eq!(count_files(&cov.join("runs")), 3);
    let runs = CoverageDir::new(&cov).read_runs().unwrap();
    assert_run(&runs[2], "invalid", commitment);
    assert!(matches!(runs[2].outcome, RunOutcome::Failed(_)), "{:?}", runs[2].outcome);
}

#[test]
fn package_without_debug_info_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let (package, _) = assemble_fixture(tmp.path());
    let stripped = tmp.path().join("stripped.masp");
    package.without_debug_info().unwrap().write_to_file(&stripped).unwrap();
    let cov = tmp.path().join("cov");

    let output = miden_cov_run(&stripped, &cov).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no debug info"), "{stderr}");
    assert!(stderr.contains("debug = true"), "{stderr}");
    assert!(stderr.contains("miden-project.toml"), "{stderr}");
    assert!(!cov.exists());
}
