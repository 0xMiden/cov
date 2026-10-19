# 0009: Input is a package that kept its debug info; no coverage build

Status: accepted (2026-10-19)

## Context

The assembler and the compiler both keep a debug-info section in the package
under the default dev profile and drop it under the default release profile
unless the project's profile says otherwise. Other ecosystems that require a
special coverage build suffer from builds that differ from production and from
users forgetting the flags.

## Decision

The CLI accepts a package and nothing else. It does not assemble MASM or
invoke the compiler. A package without a debug-info section is a hard error
whose message names the profile key that keeps it. Coverage therefore consumes
whatever the existing build produces.

## Alternatives

- Also accept MASM files or project directories and assemble them with debug
  info forced on: a second build path to maintain, and a workflow users would
  have to adopt only for coverage.

## Consequences

- The demonstration flow is: build with the usual tools, then `run`, then
  `report`.
- Users who measure release builds must set the profile key in their project
  and that is documented, not worked around.
