# 0017: Record node starts per parent edge

Status: accepted (2026-10-19). Refines 0007.

## Context

The shape matrix exposed a case the node-start model cannot express. A
procedure has one source tree however many places call it. When an `if` arm
or a loop body is a bare `exec` of a multi-block procedure, the arm's source
node is that procedure's root, and its start count includes every other call
of the procedure: from other `if`s, other loops, and direct calls. The idiom
`if.true exec.a else exec.b end` is the most common shape in MASM contracts.

A reporter-side repair, subtracting starts attributable to other parents,
works only when the other parents are joins and calls, whose child counts
equal their own. When the other parents are splits or loops the arms can only
be bounded, and the report over-counts.

## Decision

The collector records every node start twice: once per node and once per
(parent, child) edge. It knows the parent because it keeps a stack of open
nodes per forest: a start pushes, a finish pops through the node, a loop's
per-iteration finish leaves the loop open, and returning from another forest
drops the external node that led out. The parent of a starting node is the
innermost open node of the same forest that lists it as a child.

The reporter reads arm counts and loop iterations from edges. Node starts
remain for the split's own evaluation count and for function fallbacks.

## Alternatives

- Subtract other parents' starts on the reporter side: exact only when no
  other parent is a branch or a loop; rejected after the matrix showed the
  over-count.
- Duplicate source subtrees per call site in the debug info: an assembler
  change with a large size cost, and the processor would still report one
  source node per exec node.

## Consequences

- The run file format changed; its version was bumped before any release.
- The open-node stack adds a small per-start cost; per-operation cost is
  unchanged.
- When both arms of a split are the same source node, the edges coincide
  and the arms cannot be told apart; that program is pathological.
