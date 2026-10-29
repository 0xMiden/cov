# 0024: The default report scope is the source root

Status: accepted (2026-10-29). Supersedes the report-scope part of 0016 and
narrows the reporting part of 0021.

## Context

The report covered every package with debug info that a run touched,
including linked libraries whose sources resolve, and include and exclude
globs narrowed it (0016). In a user project the core library and other
dependencies resolve from the cargo registry or a toolchain sysroot, and
their files outnumber the project's. Every user had to pass the same
`--include` to see their own code, and files that did not resolve were
listed one by one, tens of lines per dependency.

## Decision

- Without include globs, a file is reported only if it resolves under the
  source root (`--source-root`, the current directory by default). Include
  globs replace the scope and may name files anywhere; exclude globs apply
  as before.
- Library coverage needs no special case: in the library's own repository
  its sources are under the checkout.
- Unresolved files are not scoped, since where they would be is unknown.
  They are summarized one line per package with the directory their paths
  share, e.g. `miden::core: 43 source files not found under
  /home/runner/work/miden-vm/crates/lib/core/asm/`, which gives the `OLD`
  side of a `--path-prefix-map`. The page and the comment carry the summary
  as one collapsed note.
- Coverage directories do not record package names, so a package is
  labelled by the namespace its own procedures share. Every program shares
  the label `programs`, so the inline programs of a test suite are listed
  once; a package with neither is labelled by the start of its commitment.

## Alternatives

- Keep the rule and have users pass `--include`: every user writes the same
  glob, and forgetting it reports the dependencies as the project.
- Exclude by package origin: the collector does not record where a package
  came from, and the map avoids any notion of "the user's package".

## Consequences

- The collector still writes every touched package (0021); the report drops
  its files when they resolve outside the source root.
- Matching a baseline and a diff to the head assumes the source root is the
  repository root, as it is in CI and for the make target.
