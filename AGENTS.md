# Working in miden-cov

Code coverage for Miden VM programs. Read `docs/architecture.md` before any
change; it is short and it defines the vocabulary used everywhere in the code.

## Documentation rules

- `docs/architecture.md` is the map. It holds no file paths, function names,
  or version numbers, and no sentence the code can confirm or refute. Keep it
  that way.
- `docs/decisions/` is the history. A change that makes a design choice adds a
  numbered record there and updates the affected section of the map in the
  same PR. Never delete or edit a record's decision; supersede it.
- The roadmap is the list of GitHub issues linked from the README. The issue
  body is the single statement of a phase's scope and of what "done" means;
  edit the body in place when the discussion changes it.
- Invariants are named in the map and enforced in code with a comment giving
  the reason. Formats and types are documented in rustdoc, next to the code.
- When a document and the code disagree, find out which is wrong before
  changing either.
- Scratch files at the repository root (`plan.md`, `design.md`,
  `handoff.md`, `review*.md`) are never committed.

## Commands

- `make lint` runs format check, clippy with warnings denied, cargo-shear, and
  the collector's no-std build.
- `make test` runs the whole test suite with nextest.
- `make format` formats with the pinned rustfmt toolchain.
