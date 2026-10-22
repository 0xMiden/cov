# 0021: A guessed source node counts nothing; a touched package is always reported

Status: accepted (2026-10-22). Restates the ambiguous-entry gap of 0018.

## Context

The processor tracks source nodes for an invocation only when it can pick
the entry's source root; a dynamically linked procedure whose root is also
another procedure's root, such as a wrapper made of one `exec`, leaves the
whole invocation untracked (the VM gap 0018 records), and a procedure it
`call`s inherits the missing node instead of being looked up. The debugger
engine then resolves each cycle to the unique source root of its node, which
is what its display wants. The collector before 0018 dropped such steps; the
engine port recorded them, and a block that is both a procedure's root and
an arm of another procedure was credited to the procedure although only the
arm ran. A coverage tool may under-report; it must not report what did not
happen.

The engine's record says when a source node was inferred rather than
tracked. One case is exact: the first cycle of an execution started from a
bare program, whose debug info the processor installs on that very step,
adopting the same root. An execution started from a package is tracked from
its first cycle.

## Decision

A cycle whose source node was inferred counts as a cycle and marks its
package as touched, but hits nothing: no node start, no edge, no operation.
The first cycle of an execution is exempt, since it is inferred only in the
exact case. Invariant: coverage may be lost, never invented. The two
exceptions to "nothing that did not happen" are attempts, not guesses, and
stand: a `call` whose callee the host cannot find counts one entry (the edge
is recorded when the child starts), and a digest shared with a callee the
report cannot see is credited to the visible function.

Every package the execution passed through is written, hit or not, so what
ran without being attributed shows as uncovered lines rather than leaving
the report.

## Alternatives

- The engine stops inferring on the stepper path. Breaks the first-cycle
  record of a bare program, and the debugger's display wants the guess.
- The engine exempts the first cycle itself. It would bake the processor's
  adoption rule into the engine for one consumer's sake; the consumer that
  must not guess holds the rule.
- Restart tracking at a `call` inside an untracked invocation, as the
  processor does for dynamic and external nodes. That is the processor's to
  change; until then the callee's exact root is lost with the rest.

## Consequences

- Below an ambiguous dynamic entry the report shows zero counts. The entry
  itself is counted from the caller's side when its root names one function,
  through the copy or the external node that enters it; an ambiguous root is
  counted nowhere, for a `call` as for anything else. The ignored reporter
  test of 0018 keeps the numbers for when the processor and the `if`-root
  follow-up close the gap.
- A package reached only through an untracked invocation is now in the
  directory, and its records take part in the identity set of the call
  rule: a root it shares with another function of its own becomes ambiguous
  across the directory, and a `call` to that function from a tracked program
  is withheld too, where it was credited when the package was not written.
- A run file's cycle count includes the cycles that hit nothing.
- The two drivers do not fail alike: the command line writes a failed run
  for an initial context the engine cannot build, while the program executor
  of 0018 runs the processor's own executor without collecting for an entry
  it cannot step from; advice inputs larger than the processor's limit still
  stop the command line before any run, as they did before. Unified when the
  executor is refined for the protocol.
- A directory written before the VM 0.36 move is rejected by the VM's own
  debug-info version message rather than the reporter's, a consequence of
  the VM move rather than of this decision; the formats of the directory are
  unchanged.
