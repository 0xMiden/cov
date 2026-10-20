# 0018: Collect coverage through the debugger engine

Status: accepted (2026-10-20). Supersedes 0002 and 0004; narrows 0005.
Lands when the engine release that carries the per-cycle hook is published;
until then the branch builds against the engine's branch.

## Context

The debugger engine already resolves, on every cycle, which source row is
about to execute: the forest, the source node, the padded operation index,
and the inline call frames. The collector did the same work in its own
observer over the processor's step API (0002), kept engine-free so that it
could follow the protocol's VM release ahead of the debugger (0004). Two
readings of the same continuation stream is one too many: every VM change to
that stream had to be understood twice, and the differential test existed
only to keep them in agreement.

Measured before the change, release build, best of five: stepping the
processor costs about ten times its raw execution on an operation-heavy
program, and the engine's full step costs about twice the collector's own
step on top of that, because it clones the operand stack twice per cycle and
refreshes the call stack, the debug variables, and the recent-operation ring.

## Decision

The engine is the only reader of the continuation stream. It produces one
record per cycle, before the cycle runs, that names the forest, the source
node, the operation and its padded index, the inline frames, and, when the
cycle starts a source node, the parent that started it (the open-node stack
of 0017 moves into the engine). Instruments receive the record through the
engine's existing instrument trait, and a stepper that owns no host drives
the processor with any synchronous or asynchronous host and hands each record
to a callback. The engine's debug executor goes through the same resolution.

The collector depends on the engine. It keeps the per-package hit tables, the
run file, and the coverage directory, and feeds them from the records. Both
entry points drive the stepper: the `run` command with the host the engine
builds from its registry, inputs file, and event handlers; the program
executor, which lets test suites plug coverage into executors generic over
the VM's program-executor trait, with the host the suite supplies.

## Alternatives

- Keep the observer and add the engine as a second backend: two
  implementations to keep in agreement, the situation this record ends.
- Make the debug executor generic over the host and drive it from the program
  executor: its state is not `Send`, which the program-executor future must
  be, and its inspection bookkeeping is dead weight for batch collection.
- Export the processor's tracer trait and hook there: a VM change with a
  release cycle of its own, and it would still leave the source resolution
  duplicated between the engine and the collector.
- Per-event callbacks instead of one record per cycle: in step mode a cycle
  has at most one node start, and one record per step is easier to test.

## Consequences

- The collector moves to a new VM release only when the debugger does. The
  clause of 0005 that let the collector pin ahead no longer holds; the
  protocol's test suites wait for the debugger when it lags.
- The run file format is unchanged. The shape matrix, the shared-arms test,
  and the differential test pass unchanged; the differential test is now the
  contract test of the engine's hook against the engine's own row
  attribution.
- Collection never goes through the debug executor, whose per-cycle
  bookkeeping costs about twice the stepper; the debug executor goes through
  the same resolution and stays the debugger's own. Rust coverage needs more
  static debug info per record, not runtime state, so adding to the record
  serves both.
- Gaps found in the VM's step API, to be raised upstream: no public way to
  rebuild the execution output from a stepped processor (the stepper clones
  the deferred state to export the witness); no way to start a stepped
  execution at a given entry-point source node, so the program executor
  starts from the unique source root; the supplied debug info is deep-copied
  on the first step of a context built from a bare program.
- The engine's debug executor gained a field, which breaks construction by
  struct literal; the debugger's own TUI did that and was updated. The rest
  of the engine change is additive.
