# 0015: Release process from the compiler, not the VM

Status: accepted (2026-10-19). Supersedes the release part of 0012.

## Context

0012 took the Makefile, CI, and release structure from the VM. The VM's
release process turned out to carry a policy tool, a set of shell scripts,
and four workflows designed for a large multi-crate workspace. For a
three-crate tool that is more machinery than value.

The compiler repository releases through a release tool driven by a small
configuration that declares release units, with three workflows for impact
checks, verification, and the release itself, and a changelog written at
release time.

## Decision

Replace the VM-derived release machinery with the compiler's, modelled on
its compiler unit only: one release unit covering the three crates and the
`miden-cov` binary, the compiler's three release workflows, and its release
process document adapted to this repository. The SDK and template units of
the compiler have no counterpart here and are not ported.

The Makefile and the lint and test workflows stay as taken from the VM.

## Alternatives

- Keep the VM process: rejected for weight.
- No release automation until the first release: would mean designing it
  under time pressure later.

## Consequences

- A release is cut the way the compiler documents it; see the release
  process document.
- Changelog entries are produced at release time; feature PRs do not touch
  the changelog.
