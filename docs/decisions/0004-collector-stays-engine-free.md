# 0004: The collector crate never depends on the debugger engine

Status: accepted (2026-10-19)

## Context

Test suites, including the protocol's own, must be able to depend on the
collector to measure coverage of what their tests execute. The debugger engine
pins exact VM versions and historically moves to a new VM release one cycle
after the protocol does. A crate that the protocol depends on and that pulls
the engine would carry two incompatible VM versions until the debugger catches
up, and the program-executor types would not match.

There is no dependency cycle; the engine depends on VM crates only. The
problem is release lockstep, and secondarily the engine's CLI and debug
adapter dependencies, which a test feature does not need.

## Decision

The collector depends on the VM crates only. The observer and the run-file
writer live there. Everything that needs the engine lives in the CLI crate.

## Alternatives

- One crate for everything: breaks the protocol's coverage feature on every VM
  release until the debugger follows.
- Put the hit counter inside the processor behind a feature: puts a
  file-writing concern into the VM; rejected in 0011.

## Consequences

- Three crates: collector, cov, cli.
- The collector may pin a newer VM than the rest of the workspace when the
  protocol requires it (see 0005).
