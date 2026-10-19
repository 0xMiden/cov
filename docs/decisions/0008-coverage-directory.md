# 0008: Two-stage workflow through a self-contained coverage directory

Status: accepted (2026-10-19)

## Context

Every Rust test is a separate VM run, under the usual test runner a separate
process, and CI shards tests across machines. The report needs the set of
coverable rows, which lives in each package's debug info, but packages used by
test suites are embedded in binaries and never exist on disk. cairo-coverage
and the Solana sBPF tool both write one raw file per run into a directory and
merge at report time.

## Decision

Collection writes into a coverage directory; reporting reads it. The directory
holds the debug-info section of each package once, keyed by package digest,
and one hit map per run keyed by package digest, source node, and row. Both
are versioned binary files using the VM's serialization traits. The reporter
never needs the original packages. A `dump` subcommand prints a run as JSON for
inspection. Default location is under the Cargo target directory, overridden
by an environment variable.

## Alternatives

- One JSON file per run containing the full denominator: inspectable, but
  repeats the debug info per run and is large for the core library.
- Reference packages by path: fails for embedded packages.
- In-process aggregation with a flush at exit: needs global state and an exit
  hook; per-run files are simpler and merge the same way.

## Consequences

- Concurrent writers of the same package file must be harmless; writing the
  identical content twice is acceptable.
- Format versions are a compatibility contract; an unknown version is refused
  with a message, never guessed.
