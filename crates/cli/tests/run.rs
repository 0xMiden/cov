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

/// Assembles the fixture `tests/fixtures/<name>.masm` from its path, so debug
/// locations carry the real file path, and writes it to `dir`. Returns the
/// package and its file.
fn assemble(dir: &Path, name: &str) -> (Package, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.masm"));
    let package = *Assembler::default().assemble_program(name, source).unwrap();
    let path = dir.join(format!("{name}.masp"));
    package.write_to_file(&path).unwrap();
    (package, path)
}

/// Assembles `fixture.masm` with [`assemble`].
fn assemble_fixture(dir: &Path) -> (Package, PathBuf) {
    assemble(dir, "fixture")
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

/// A program whose advice map gives a key another value than the inputs file
/// does: the execution context cannot be built, so the program fails before
/// its first cycle, which is what the test pins; any failure of building the
/// context looks the same from here. The run is still written, as a failed
/// run of no cycles that passed through no package. Returning the error
/// before writing would leave the directory empty and lose the failure from
/// the report.
#[test]
fn a_run_whose_context_cannot_be_built_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let (package, masp) = assemble(tmp.path(), "adv_map");
    let cov = tmp.path().join("cov");
    let (key, _) = package.mast_forest().advice_map().iter().next().unwrap();
    let inputs = tmp.path().join("conflict.toml");
    fs::write(
        &inputs,
        format!("[[inputs.advice.map]]\ndigest = \"{}\"\nvalues = [2]\n", key.to_hex()),
    )
    .unwrap();

    let output = miden_cov_run(&masp, &cov).arg("--inputs").arg(&inputs).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.starts_with("wrote "), "stdout: {stdout}\nstderr: {stderr}");
    let runs = CoverageDir::new(&cov).read_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert!(matches!(runs[0].outcome, RunOutcome::Failed(_)), "{:?}", runs[0].outcome);
    assert_eq!(runs[0].cycles, 0);
    assert!(runs[0].packages.is_empty(), "{:?}", runs[0].packages);
    // A missing directory reads as no packages; any other error fails the test.
    assert!(
        CoverageDir::new(&cov).read_packages().unwrap().is_empty(),
        "no package is written"
    );
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

#[test]
fn report_merges_runs_into_lcov_and_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, masp) = assemble_fixture(tmp.path());
    let cov = tmp.path().join("cov");
    for (name, stack) in [("taken", [1, 3]), ("skipped", [0, 0])] {
        let inputs = write_inputs(tmp.path(), &format!("{name}.toml"), stack);
        let output = miden_cov_run(&masp, &cov)
            .arg("--inputs")
            .arg(&inputs)
            .args(["--name", name])
            .output()
            .unwrap();
        assert_wrote(&output);
    }

    let lcov = tmp.path().join("out/lcov.info");
    let output = Command::new(env!("CARGO_BIN_EXE_miden-cov"))
        .arg("report")
        .arg("--cov-dir")
        .arg(&cov)
        .arg("--lcov")
        .arg(&lcov)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    eprintln!("{stdout}");
    assert!(stdout.contains("fixture.masm"), "{stdout}");
    assert!(stdout.contains("TOTAL"), "{stdout}");
    assert!(stdout.contains(&format!("wrote {}", lcov.display())), "{stdout}");

    let trace = fs::read_to_string(&lcov).unwrap();
    // Both arms of the if and both ways through the loop guard were taken.
    assert!(trace.contains("BRDA:15,"), "{trace}");
    assert!(trace.contains("BRDA:7,"), "{trace}");
    assert!(!trace.contains(",-\n"), "{trace}");
    // The loop body ran six times: n = 3 doubled.
    assert!(trace.contains("DA:8,6\n"), "{trace}");
    // The else arm ran once, in the skipped run.
    assert!(trace.contains("DA:18,1\n"), "{trace}");
    assert!(trace.contains("FNDA:1,::$exec::double\n"), "{trace}");
    assert!(trace.contains("FNDA:2,::$exec::count_down\n"), "{trace}");
    assert!(trace.ends_with("end_of_record\n"), "{trace}");

    // genhtml must accept the trace without complaint.
    let html = tmp.path().join("html");
    match Command::new("genhtml")
        .arg("--branch-coverage")
        .arg("--output-directory")
        .arg(&html)
        .arg(&lcov)
        .output()
    {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stderr}");
            assert!(!stderr.contains("WARNING"), "{stderr}");
            assert!(html.join("index.html").exists());
        },
        Err(e) => eprintln!("genhtml not available, skipping HTML check: {e}"),
    }

    // Against itself as the baseline, with a change to the `if`. The
    // diff's path is relative to the source root, the current directory.
    let diff = tmp.path().join("change.diff");
    fs::write(
        &diff,
        "+++ b/tests/fixtures/fixture.masm\n@@ -14,0 +15,4 @@\n+    if.true\n+        exec.double\n+    \
         else\n+        push.7 drop\n",
    )
    .unwrap();
    let page = tmp.path().join("out/page.html");
    let comment = tmp.path().join("out/comment.md");
    let output = Command::new(env!("CARGO_BIN_EXE_miden-cov"))
        .arg("report")
        .arg("--cov-dir")
        .arg(&cov)
        .arg("--baseline")
        .arg(&lcov)
        .arg("--diff")
        .arg(&diff)
        .arg("--html")
        .arg(&page)
        .arg("--markdown")
        .arg(&comment)
        .args(["--page-url", "https://example.com/a"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains(&format!("wrote {}\n", page.display())), "{stdout}");
    assert!(stdout.contains(&format!("wrote {}\n", comment.display())), "{stdout}");
    let html = fs::read_to_string(&page).unwrap();
    assert!(html.contains("<section id=\"file-tests/fixtures/fixture.masm\">"), "{html}");
    let row = html
        .lines()
        .find(|line| line.starts_with("<tr id=\"file-tests/fixtures/fixture.masm-L16\""))
        .unwrap();
    assert!(row.contains("<td>+</td><td>        exec.double</td>"), "{row}");
    let markdown = fs::read_to_string(&comment).unwrap();
    assert!(markdown.starts_with("## Coverage\n"), "{markdown}");
    assert!(
        markdown.contains(
            "[<code>tests/fixtures/fixture.masm</code>]\
             (https://example.com/a#file-tests/fixtures/fixture.masm)"
        ),
        "{markdown}"
    );
    assert!(markdown.contains("| Lines | "), "{markdown}");
    assert!(markdown.contains(") | 0.0% |\n"), "{markdown}");
    assert!(markdown.contains("| 2/2 (100.0%) |"), "{markdown}");

    // A baseline needs a page or a comment to go into, and a missing one is
    // named.
    let report_with_baseline = |baseline: &Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miden-cov"));
        command.arg("report").arg("--cov-dir").arg(&cov).arg("--baseline").arg(baseline);
        command
    };
    let output = report_with_baseline(&lcov).output().unwrap();
    assert!(!output.status.success());
    let missing = tmp.path().join("missing.info");
    let output = report_with_baseline(&missing).arg("--markdown").arg(&comment).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!("`{}`", missing.display())), "{stderr}");

    // Filters drop files from the report, and malformed globs are rejected.
    let report_with = |flag: &str, glob: &str| {
        Command::new(env!("CARGO_BIN_EXE_miden-cov"))
            .arg("report")
            .arg("--cov-dir")
            .arg(&cov)
            .args([flag, glob])
            .output()
            .unwrap()
    };
    let output = report_with("--exclude", "**/fixture.masm");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(!stdout.contains("fixture.masm"), "{stdout}");
    let output = report_with("--include", "src/[");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid glob `src/[`"), "{stderr}");

    // dump prints the run as JSON.
    let run_file = fs::read_dir(cov.join("runs")).unwrap().next().unwrap().unwrap().path();
    let output = Command::new(env!("CARGO_BIN_EXE_miden-cov"))
        .arg("dump")
        .arg(&run_file)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["outcome"], "ok");
    assert!(json["cycles"].as_u64().unwrap() > 0);
    assert_eq!(json["packages"].as_array().unwrap().len(), 1);
}
