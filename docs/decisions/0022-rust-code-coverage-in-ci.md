# 0022: Rust code coverage in CI, keyed by report name

Status: accepted (2026-10-28).

## Context

The repository measures the coverage of Miden programs. Its own Rust code
had no coverage measurement. The debugger repository measures its Rust code
with `cargo llvm-cov`, renders the report with octocov, and posts it on pull
requests through artifacts: the pull request's run renders the report with a
read-only token, a job with a write token posts it for same-repository pull
requests, and a scheduled job posts it for forks and Dependabot, whose runs
have no write token.

0010 reserves octocov for the coverage of Miden programs on pull requests.
Two octocov reports in one repository would share one baseline artifact and
one comment unless they are told apart.

## Decision

Take the debugger's setup as it is on its development branch, with two
adaptations:

- Every name a report owns (report file and artifact, octocov configuration,
  baseline artifact, comment artifact, comment marker) carries a short
  report name. The Rust code is the `rust` report. The scripts take the
  name as a parameter; the Miden-program report of 0010 adds a name, not a
  copy of the scripts.
- The test runner is nextest, so the measurement uses the same test
  profiles as the test suite. Integration tests and their fixtures are
  excluded from the measured set.

The coverage is reported, not enforced: below the acceptable threshold the
report carries a warning and CI does not fail.

## Alternatives

- A coverage job in the test workflow: the comment jobs need write
  permissions the test workflow does not have, and the scheduled publisher
  finds runs by workflow file.
- Copy the scripts with fixed names and split them when the second report
  arrives: the split touches the publishing code twice.
- A coverage service (Codecov, Coveralls): octocov is already the choice of
  0010 and needs no account.
- Fail CI below the threshold: a report below the line is still useful, and
  a hard gate invites threshold games.

## Consequences

- `make coverage` writes the Cobertura report locally; the coverage
  workflow renders and posts it.
- The second octocov report of 0010 reuses the scripts and workflows with
  its own name and configuration file.
- The baseline is the default branch's report stored as an artifact; a pull
  request against another branch compares against that branch's report
  when one exists, else the default branch's.
