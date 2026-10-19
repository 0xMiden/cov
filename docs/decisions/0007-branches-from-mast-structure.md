# 0007: Branch coverage from split and loop nodes, in the MVP

Status: accepted (2026-10-19)

## Context

Foundry derives branches by matching jump opcode patterns and has a long bug
history from it. Move derives them from a control-flow graph. Miden's MAST is
structured: every `if.true` is a split node with two child source nodes and
every `while.true` is a loop node with a body, so branch identity is exact and
needs no analysis. Per-node hits fall out of the same step loop that collects
rows.

## Decision

Record source-node starts alongside row hits. A split arm is taken when its
child node starts. A loop body is entered when it starts; a loop is skipped
when its start is followed by its end, which the collector detects with a
one-step lookahead. Emit both as branch records anchored to the introducing
row's line. Ship this in the MVP.

## Alternatives

- Lines and functions only in the MVP: saves about a day and loses the most
  useful signal; nothing in the data model gets simpler.

## Consequences

- The collector keeps a small amount of state across steps for loops.
- PR comments from octocov show line coverage only; branch coverage shows in
  genhtml and in editor gutters.
