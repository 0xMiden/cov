# 0023: miden-cov renders the pull request report

Status: accepted (2026-10-29). Supersedes the pull request part of 0010 and
its note, and the consequence of 0022 on a second octocov report.

## Context

0010 left the pull request comment, the HTML page and its storage to octocov
reading the LCOV. octocov parses only the `SF` and `DA` records of LCOV, so
branch and function coverage never reach its comment or its page. For MASM
that loses an arm with no instructions of its own (the empty `else`, an arm
that only `exec`s or `call`s), a loop never skipped on entry, and the branch
and function totals with their deltas.

## Decision

miden-cov renders both parts of the `miden` report from its own model. LCOV
stays the primary output.

- `report --html FILE` writes one self-contained page: inline CSS, no
  external references, every reported file whole. Files the change touched
  come first, then files whose coverage moved against the baseline, then the
  rest. Changed lines are marked, branch arms are labelled the MASM way
  (`if` true/false, `while` iterations/exits, not evaluated apart from not
  taken), and every file and line has an anchor.
- `report --markdown FILE` writes the comment: line, function and branch
  totals, base to head with the delta; the changed and moved files with
  patch coverage, each linking to its anchor in the page (`--page-url URL`);
  a collapsed list of functions no longer entered.
- `--baseline LCOV` reads the base branch's LCOV, matching functions by name
  so the delta survives moved lines. `--diff FILE` reads a unified diff for
  the changed lines and patch coverage. Without them both outputs are
  head-only.

Size: files are drawn whole, with one guard of 16 MiB of drawn source; past
it the remaining files are listed as not drawn and stay in the totals.
octocov draws changed hunks only, capped at 1 MiB per file and 4 MiB in all;
Miden code is small, so whole files and a guard are enough. The comment stays
under the 60 KiB cap of the publishing scripts by shortening long lists to an
"N more" line.

## Alternatives

- octocov as is: loses what the context lists.
- octocov as a Go library: the comment and baseline parts can be imported,
  the single-page renderer (a JavaScript bundle run in SpiderMonkey) cannot,
  and it has no hook for per-line branch marks. Eleven exported signatures
  changed between v0.80.0 and v0.83.4, in five weeks. A fork or an upstream
  pull request was ruled out: upstream prefers custom metrics for branches
  (issue #810) and closed an LCOV-branch pull request (#811).
- genhtml with differential coverage (`--baseline-file`, `--diff-file`): it
  writes a multi-file site, and an unzipped artifact is a single file.
  Merging the site into one page worked in a prototype but was judged
  fragile; one artifact per page breaks genhtml's navigation, since artifact
  URLs are known only after upload. lcov 2.5 also rejects a hit line none of
  whose branches was taken.
- Codecov or Coveralls: they need an account; 0022 rejected them.

## Consequences

- The cov crate owns an HTML writer, an LCOV reader, a diff reader and the
  comparison. The page embeds untrusted source text and escapes all of it.
- The `miden` report has no octocov configuration; it reuses the publishing
  scripts of 0022 under its name.
- Lost coverage on unchanged lines is not shown line by line; the file and
  function deltas and patch coverage catch most regressions.
