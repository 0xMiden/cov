# Roadmap

Phases are ordered by priority. Each phase lists its scope, its acceptance
criteria, and the decision records it rests on. Pending upstream changes live
here because they churn; the architecture map stays free of them.

## Phase 1: MVP, pure MASM program

Scope:
- `miden-cov run <package.masp> [--inputs inputs.toml] [--entrypoint ...] [-l ...]`
  loads and links the package through the debugger engine, executes it under
  the collector, and writes a run file into the coverage directory.
- `miden-cov report [--lcov <path>] [--source-root ...] [--path-prefix-map old=new]`
  merges the runs, resolves sources, prints a per-file summary table, and
  writes LCOV with line, function, and branch records.
- `miden-cov dump <run-file>` prints a run as JSON for debugging.
- A package without a debug-info section is a hard error naming the profile
  key that keeps it.

Acceptance:
- A fixture MASM project with two procedures, an `if.true`/`else`, and a
  `while.true` runs with two different inputs; the merged LCOV shows the
  expected hit counts, the untaken arm as uncovered, and the skipped loop body
  as a branch with zero hits. Checked by an integration test.
- `genhtml` renders the LCOV without warnings; the README documents the step.

Decisions: 0001-0010, 0012.

## Phase 2: in-test collection for protocol, VM, and user contract tests

Scope:
- The collector's program-executor implementation, activated by the coverage
  directory environment variable, one run file per VM execution, test name
  recorded from the test thread.
- Measure per-cycle overhead of the step loop on a realistic transaction;
  decide whether the exported tracer (below) is required before this phase
  ships.
- Documentation for enabling coverage in a user contract project and in the
  protocol workspace.

Upstream changes required, each a small PR:
- VM: export the per-cycle tracer trait and add a source-aware tracer entry
  point, so the collector can drop the one-step-at-a-time loop.
- VM: the test-utils loader must keep library debug info when loading
  libraries into the host, so core-library coverage is attributable.
- Protocol: a `coverage` feature on the testing crate that wires the
  collector's executor into the mock transaction and the code executor when
  the environment variable is set.

Acceptance:
- Running the protocol's standards tests with the variable set produces a
  coverage directory whose report attributes hits to the standards MASM
  sources, with the protocol checkout as the source root.
- Running the VM's core-library tests likewise reports core-library coverage.
- The template user project from the compiler reports coverage of its account
  component.

Decisions: 0002, 0004, 0011.

## Phase 3: Rust programs

Scope:
- Packages built by the compiler in a profile that keeps debug info are
  measured by the same pipeline; locations point at Rust files.
- Coverable filtering: rows whose instruction is stack bookkeeping
  (`dup`, `swap`, `movup`, `movdn`, `drop`, padding) are excluded from line
  attribution, following the practice of other VM-level coverage tools.
- Attribution through inlining: a hit on a row whose source is a callee
  reached through inlined calls also credits the call sites, using the inline
  call rows of the debug info.
- Paths under the Rust standard library and compiler intrinsics are excluded
  by default.

Acceptance:
- A Rust fixture with a branch and a loop reports line and branch coverage on
  the Rust source, with no phantom hits on lines that contain only bookkeeping.

Decisions: 0006 (filtering clause), plus a new record for inlining credit.

## Phase 4: agent-facing output

Scope:
- `miden-cov report --json` emitting uncovered regions grouped by function,
  each with a short source excerpt, plus which tests touched each file, sized
  for a context window.
- A skill or equivalent instruction set that tells an agent writing tests how
  to run coverage and read the output.

Acceptance:
- An agent given only the JSON can name the function and lines a new test
  should reach and the existing test closest to it.

Decisions: 0014.

## Phase 5: GitHub integration

Scope:
- octocov configuration and workflow for PR comments with the coverage delta
  against the base branch.
- An HTML renderer showing the diff of the PR with covered and uncovered
  markers on changed lines, hosted so that the PR comment can link to it.
- Hosting target to be decided when the phase starts (Pages, artifacts, or a
  bucket).

Acceptance:
- A PR to this repository that changes a fixture gets a comment with the
  coverage delta and a link to the rendered diff.

Decisions: 0010, plus a new record for hosting.

## Open questions

- Per-cycle overhead of stepping versus a tracer; budget for running the whole
  protocol suite under coverage in CI.
- Whether to key run files by MAST root digest of the entry point in addition
  to the package digest, for per-entrypoint attribution.
- Hosting target for the HTML in phase 5.
