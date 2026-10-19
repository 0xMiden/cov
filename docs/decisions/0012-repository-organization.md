# 0012: Crates under `crates/`; VM-style Makefile, CI, and release; compiler-style Cargo conventions

Status: accepted (2026-10-19)

## Context

The Miden repositories differ in layout. The compiler keeps crates at the
root with an explicit member list and strict workspace-inheritance
conventions; the VM keeps most crates under `crates/` with a plain Makefile,
split lint and test workflows, and its own release workflows. The compiler
pins a nightly toolchain for wasm targets, which this project does not need.

## Decision

- All crates live under `crates/`, one directory per crate, named without the
  `miden-cov-` prefix.
- Cargo conventions from the compiler: explicit member list, inherited
  workspace package fields, grouped workspace dependencies, the same
  profiles.
- Makefile targets, CI workflow names, and release structure from the VM.
- Stable pinned toolchain; nightly only for rustfmt, as the VM does.

## Alternatives

- Mirror the compiler's cargo-make and nightly toolchain: more machinery than
  a three-crate workspace needs.

## Consequences

- Contributors from either repository find familiar commands.
- Release automation follows the VM's; when it changes there, it changes here.
