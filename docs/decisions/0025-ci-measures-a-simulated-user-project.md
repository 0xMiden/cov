# 0025: CI measures a simulated user project

Status: accepted (2026-10-29).

## Context

Issue #5 planned to post the coverage of the repository's `.masm` fixtures
run through `miden-cov run`. That is the proof-of-concept path, not how
on-chain code is tested: on-chain code is always executed by off-chain code
that prepares the environment (kernel, advice inputs, event handlers, linked
libraries), as the protocol's tests do. The workflow rehearses how the
protocol will measure its MASM (#2); its shape matters more than its numbers.

## Decision

CI measures `examples/wallet`, an unpublished workspace member: a kernel
reached by `syscall`, a wallet library with conditionals, an assertion with
an error code, an advice-map read and an event, and a note library that
`call`s the wallet. Its tests prepare the environment through a copy of the
protocol's `CodeExecutor` helper that runs programs on the collector's
`CoverageExecutor`, and assert program outputs only. Libraries are loaded as
packages, since a bare forest drops the debug info. The tests run in the
normal suite without `MIDEN_COV_DIR`, which also checks that coverage off
changes nothing.

`make miden-coverage` holds the sequence, so it runs the same locally: the
example's tests with `MIDEN_COV_DIR` set, then `miden-cov report` for LCOV
and the page, failing when the report has no hits. `make
miden-coverage-comment` renders the comment. Baseline, diff and page URL are
make variables.

The workflow `miden-coverage.yml` produces the `miden` report, a name that
covers Rust and MASM on-chain code later:

- It runs on pushes to `main` and `next` and on pull requests, with no path
  filter, which would skip most pull requests and let the baseline go stale.
- `miden-cov` is built from the pull request's own source, so a change to
  the reporter shows in its own report.
- Push runs store the LCOV as the baseline. A pull request takes the LCOV of
  the newest successful push run on its base branch, else on the default
  branch (0022's rule); with none, the comment is head-only.
- The diff is the merge commit against its first parent.
- The page is uploaded as one unzipped artifact; the comment links to its
  file anchors.
- The comment is posted by the publishing scripts of 0022, forks included.
- No threshold: the report is informational.

## Alternatives

- The fixtures through `miden-cov run`: the proof-of-concept path above.
  The fixtures and `run` stay as test inputs.
- `.masp` files assembled with the midenup toolchain: `miden-vm compile`
  writes a bare MAST program without debug info, and `bundle` builds
  libraries only and cannot link our own library packages.
- The existing tests honoring `MIDEN_COV_DIR`: they are edge-case shapes in
  temporary directories, so the numbers would mean nothing.
- A separate cargo workspace for the example: a second lockfile duplicating
  the pins, with nothing to keep it in step.

## Consequences

- A pull request gets two coverage comments, `rust` and `miden`; the `rust`
  report ignores `examples/`.
- The example shows the shape #2 gives the protocol's own helper.
- Baselines expire with their artifacts; a pull request then gets a
  head-only comment.
