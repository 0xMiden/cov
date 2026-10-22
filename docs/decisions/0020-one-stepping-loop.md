# 0020: One stepping loop in the engine

Status: accepted (2026-10-22). Amends the consequences of 0018: the
differential test is removed and the debug executor is a consumer of the
stepper.

## Context

0018 moved the collector onto the debugger engine's per-cycle record, which
the engine's stepper produces, and left the engine's own debug executor with
a stepping loop of its own over the same source resolution. Two loops over
one resolution are two lifecycles to keep in sync, and the only check that
they agreed was the collector's differential test, which fed the same
program through both and compared the attributions. Both sides of that test
read the cycle from the same resolution, so it could not catch a wrong
attribution, only a loop that dropped or doubled a record.

## Decision

The debug executor steps through the stepper. The stepper shows the
executor what a step is about to run before the cycle, and the executor
brings its call stack and declarations up to date from that; everything the
executor keeps for inspection is refreshed from the processor after the
cycle. There is one place that takes a context, describes the cycle, runs
the open-node tracking, and dispatches the step.

The differential test is removed, with the example that benchmarked the
debug executor's path against the stepper's and the programs both read.
With one loop there is no second reading to compare, and the example's
"engine" row measured a path that no longer exists apart from the stepper.

## Alternatives

- A shared step core used by both loops (take the context, describe the
  cycle, run the tracker, dispatch the step), each keeping its own lifecycle
  and the debug executor its public fields. A smaller change, but the two
  state machines remain, and so does the question which one is right when
  they disagree.
- Keeping the differential test as a pipeline test of the stepper against
  the debug executor. With the executor a consumer of the stepper it would
  compare a loop with itself.

## Consequences

- The attribution tests of this repository are the ones with hand-written
  expectations: the shape matrix, the shared-arms test, and the call and
  inlined-copy tests. Nothing checks the engine's resolution against an
  independent reading; a wrong resolution shows up as a wrong expected count.
- The CLI path (`miden-cov run`) is covered by its own tests against a
  fixture; the end-to-end comparison of the CLI with a direct collection that
  lived in the differential test is gone with it.
- The overhead numbers of 0018 stand as measured on the branch; the example
  that produced them is not kept.
- The debug executor's per-cycle cost is the stepper's plus its own
  bookkeeping, and any change to how a cycle is described reaches both the
  collector and the debugger at once.
