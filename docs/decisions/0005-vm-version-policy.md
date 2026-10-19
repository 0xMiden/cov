# 0005: Published releases only; the workspace follows the debugger's VM pin

Status: accepted (2026-10-19)

## Context

The compiler, debugger, and protocol each pin a VM release, and at any moment
they can differ by one. The CLI cannot move ahead of the debugger engine
(0003). The collector must match the VM release the protocol uses (0004).

## Decision

All Miden dependencies are crates.io releases; no git pins. The workspace
tracks one VM release: the one the debugger engine pins. When the protocol
moves to a VM release the debugger does not yet support, the collector crate
alone may pin ahead, since test suites need the collector and not the CLI.
Bump the workspace as soon as the debugger does.

## Alternatives

- Follow the protocol everywhere: the CLI would not compile against the
  engine.
- Git pins on `next` branches: unreproducible builds and release pain.

## Consequences

- At most two VM versions in the lockfile, and only during a lag window.
- A VM release that changes the debug-info format forces a collector release
  the same day.
