# 0006: Rows and source nodes are the unit; hit counts, not booleans

Status: accepted (2026-10-19)

## Context

The debug info has one row per MASM instruction, with the index and count of
the VM operations it expanded to, and a tree of source nodes that mirrors the
MAST one source occurrence at a time. The VM reports the current source node
while stepping, so attribution is exact even when exec nodes are deduplicated.

Other tools either count every executed operation, which inflates lines that
compiled to many operations, or collapse consecutive operations of the same
line, which is a heuristic.

## Decision

- The unit of collection is the row and the source node.
- A row with operations is hit when its first operation executes. A row with
  no operations is hit when its source node starts.
- A line's count is the maximum over its rows. A function's count is the
  start count of its root source node.
- Counts are stored; truncation to booleans is an output option.
- In a MASM package every row is coverable. Filtering of bookkeeping
  instructions applies to Rust sources only and is a later phase.

## Alternatives

- Count operations: inflates multi-operation instructions.
- Collapse consecutive runs: a heuristic that MASM, with one instruction per
  line in practice, does not need.
- Booleans: lose per-test attribution and cost nothing to avoid.

## Consequences

- The collector must know where each row's first operation is; the reporter
  must know each row's location and node.
- The rule is specified in the architecture map and implemented across two
  crates; tests must cover both ends.
