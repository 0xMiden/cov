# 0011: Test suites activate coverage through a feature plus an environment variable

Status: accepted (2026-10-19); the VM-pin consequence is superseded by [0018](0018-collect-through-the-debugger-engine.md)

## Context

The protocol's transaction executor is generic over a program-executor trait,
which is the right seam for a collecting executor. But the testing crate's
helpers, used by hundreds of call sites in the protocol and by user contract
tests, construct the default executor themselves. The VM's own core-library
tests go through a test-utils crate with the same shape of problem.

## Decision

The testing crate gains an optional `coverage` feature that depends on the
collector and, when the coverage directory environment variable is set, wires
the collector's executor into the mock transaction and the code executor.
The VM test-utils loader is changed to keep library debug info. With the
feature on and the variable unset, nothing changes. Each VM run writes its own
run file; no state is shared between tests.

## Alternatives

- Generic setters only, with every test wiring the collector: does not scale
  to the existing call sites.
- A built-in hit counter in the processor behind a feature: makes the VM write
  files and know about coverage directories.
- A global registry in the collector flushed at process exit: needs an exit
  hook and breaks under test runners that kill processes.

## Consequences

- Two upstream PRs, listed in #2.
- The collector's executor must satisfy the protocol's trait exactly, which
  ties the collector's VM pin to the protocol's (see 0005).
