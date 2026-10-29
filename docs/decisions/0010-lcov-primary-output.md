# 0010: LCOV is the primary output; octocov for PR comments

Status: accepted (2026-10-19); the pull request part and its note are superseded by [0023](0023-miden-cov-renders-the-pull-request-report.md)

## Context

LCOV is read by genhtml, octocov, Codecov, Coveralls, and editor coverage
gutters, and it carries line, function, and branch records. The other formats
in use (Cobertura, Clover, JaCoCo, Istanbul JSON, Codecov JSON) each serve one
consumer. The Miden repositories already use octocov for PR coverage comments.

## Decision

The MVP writes LCOV and a terminal summary only. HTML comes from genhtml until
the GitHub phase, where a diff-aware renderer is needed anyway. Pull request
comments come from octocov reading the LCOV; miden-cov's contribution to the
PR is the link to the hosted HTML diff. Other formats are added only when a
consumer needs them.

## Note added the same day

octocov renders the HTML and stores it as a workflow artifact, so no
renderer or hosting is built here. The HTML diff part of the decision is
dropped; the rest stands. Tracked in #5.

## Alternatives

- Own HTML from day one: duplicates genhtml for the MVP's purpose.
- Cobertura first: octocov reads it too, but nothing else in the toolchain
  prefers it.

## Consequences

- Branch coverage is visible in genhtml and editors; octocov shows lines.
- ~~Phase 5 adds the renderer and hosting.~~ Dropped, see the note above.
