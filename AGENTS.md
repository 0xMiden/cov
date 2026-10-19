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
- `docs/roadmap.md` is the future. Update the phase you are working on.
- Invariants are named in the map and enforced in code with a comment giving
  the reason. Formats and types are documented in rustdoc, next to the code.
- When a document and the code disagree, find out which is wrong before
  changing either.
- Scratch files at the repository root (`plan.md`, `design.md`,
  `handoff.md`, `review*.md`) are never committed.

## Commands

- `make lint` runs format check and clippy with warnings denied.
- `make test` runs the whole test suite with nextest.
- `make format` formats with the pinned rustfmt toolchain.

## Conventions

- Branch from `next`; `main` holds releases.
- Conventional commits with a scope: `feat(collector): ...`,
  `fix(report): ...`, `docs: ...`.
- Document every module, type, and public function. Say what, not how.
- Changelog entries are generated from commit messages at release time; do
  not edit `CHANGELOG.md` in feature PRs.
