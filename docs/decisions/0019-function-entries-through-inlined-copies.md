# 0019: Count function entries through inlined copies

Status: accepted (2026-10-21). Refines 0016; narrows the premise of 0017.

## Context

The VM release the engine moved to changes the shape of the debug info. The
assembler now emits a copy of a procedure's source tree at every place the
procedure is inlined, and marks every row of a copy with an inline call
naming the procedure and the call site. The procedure's own tree, the one its
function record points at, executes through a `call`, as the entry point, or
as a dynamic target; the copies do the rest. Before, an inlined procedure had
one tree and its root started once per call.

Most of the reporter did not notice: rows are aggregated by location (0016),
and copies of a row share one location. What broke is the function count of
a procedure that has no coverable row of its own, such as a body made only of
`exec` instructions, which fell back to the starts of the function's root.
That root is not started by an `exec`.

## Decision

The fallback counts, besides the root's own starts, every entry into an
inlined copy. A copy is entered where its inline call begins: at a node
whose parent does not carry the same call (same callee and call site), or in
the middle of a block. On a block the entry counts once per execution of the
first operation of the call's range; on a control node, which has no
operations, once per start of the node.

A `call` inside such a copy clones the callee's tree too, without an inline
call naming the callee: the clone's root is the `call` node's child, shared
by every `call` of the callee within the copy, and each `call` node enters it
once per edge from the node to the child. It is credited to the one function
whose MAST root it is, looked up across the packages of the run: a `call`
into a linked library names a procedure whose record lives there and whose
root start the collector never sees, so the edges are its only count. A root
shared by several functions is skipped: two procedures
whose bodies compile to the same MAST, such as a procedure made of one
`exec` of another, cannot be told apart by their roots, and the entries of
such a procedure through a `call` inside an inlined copy are not counted.
The assembler should mark cloned call targets like inlined copies, which
would close that gap; to be raised upstream.

The primary rule stays: a function with a coverable row of its own counts
the hits of its first such row, which also sees bodies entered from another
package, whose starts the collector never observes.

Edges (0017) stay. Inlined arms have their own nodes, and so does every
`call` node; what the `call`s of one procedure within one copy share is the
clone, which is why their entries are edges. The assembler also records a
re-exported procedure twice, differing only in the linkage name; the reporter
folds the duplicates into the first record before it counts.

The call rule rests on the collector never seeing the root start of a
procedure entered from another package: if the engine ever described it, the
library's root starts and the caller's edges would both count the entry.

## Alternatives

- Identify copies by exec node, counting the starts of every source node
  whose exec node is the function's root: conflates procedures with
  identical bodies, which the assembler deduplicates into one exec node.
- Make the inline entries the primary rule: misses bodies entered from
  another package, and the row rule already covers inlined copies.

## Consequences

- The reporter builds a parent map and reads the inline-call rows once per
  package; cost proportional to the debug info, not to the run.
- Rust coverage gets the same mechanism for free: the inline call rows are
  how the compiler describes inlined Rust functions.
