# 0002: Observe through the processor's step API until the tracer trait is exported

Status: accepted (2026-10-19)

## Context

The processor has a trait that is called at the start and end of every clock
cycle with the continuation about to execute, which is the ideal hook. Its
module is private in every published release, so no external crate can
implement it. The public alternative is the step API: build a resume context
from a package, step one cycle at a time, and inspect the next continuation
and source node before each step. The debugger is built on it.

## Decision

The collector observes execution through the step API. The observation itself
is a function over the resume context, kept separate from the loop that drives
stepping, so that the same observer can be driven by the debugger engine's
executor in the CLI and by the collector's own program executor in test
suites.

In parallel, propose a VM change that exports the tracer trait and adds a
source-aware tracer entry point. Once released, the collector switches its
backend without changing the observer or the run file.

## Alternatives

- Wait for the VM change before building: blocks the MVP on a release cycle
  for no gain in the data collected.
- Fork or vendor the processor: unacceptable maintenance cost.

## Consequences

- Per-cycle overhead of stepping is unmeasured; #2 requires a measurement
  before in-test collection ships.
- The backend is an internal seam; nothing outside the collector may depend on
  which one is in use.
