# 0003: The CLI uses the debugger engine for loading, linking, and inputs

Status: accepted (2026-10-19); the observer sentence and the reference to 0004 are superseded by [0018](0018-collect-through-the-debugger-engine.md)

## Context

Running a standalone package needs package loading with format-version
checks, a registry that finds the core and protocol libraries from a sysroot
and link flags, an inputs file for stack and advice data, and an entry point
selector for library packages. The debugger engine provides all of this and
users of the debugger already know its flags and inputs file.

## Decision

The `run` subcommand of the CLI is built on the debugger engine: its package
loading, its registry, its inputs file format, and its executor. The CLI
mirrors the debugger's flag names one to one. The collector's observer is
driven by the engine's executor.

## Alternatives

- Reimplement loading and linking on the processor directly: about as much
  code as the whole MVP, and a second inputs format for users to learn.
- Depend on the engine everywhere, including the collector: rejected in 0004.

## Consequences

- The CLI's VM version is the one the engine pins (see 0005).
- Changes to the debugger's flags or inputs format are followed here.
