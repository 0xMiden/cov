# 0016: Aggregate hits by location; count functions by their first instruction

Status: accepted (2026-10-19). Refines 0006 and 0007.

## Context

Running a real program against the reporter showed four attribution errors
that the single-file fixture could not reveal:

- Every join node inside a procedure, including joins inside loop bodies,
  carries a zero-operation `begin` row located at the whole procedure span,
  so the definition line showed the loop iteration count.
- The assembler inlines a single-block procedure into each caller's basic
  block, and the copied operations keep rows pointing at the callee's lines.
  Taking the maximum over rows undercounted a callee reached from several
  sites, and a callee inlined twice in one block counted once.
- A procedure that is only ever inlined, or a library procedure entered
  through an external node, never starts its own root node, so its function
  count was zero or far too low.
- The guard split of a loop only records "skipped on entry", so a loop that
  iterated and then exited looked as if its false arm was never taken.

## Decision

- Zero-operation rows are coverable only on split nodes. Those on join and
  loop nodes are structural duplicates and are ignored.
- Hits are aggregated per exact location: rows with the same file and byte
  range are copies of one instruction and their counts are summed. A line's
  count is the maximum over the locations credited to it, since one line may
  hold several instructions.
- A function's count is the count of the first coverable location in its
  body that carries the function's own procedure path, in execution order,
  stopping at the first conditional or loop. The procedure path is needed
  because an inlined procedure's source node is the block of its call site,
  and a body may begin with rows of a procedure inlined into it. This counts
  inlined copies and externally entered bodies alike. Node starts are the
  fallback when no such row exists.
- The branch at a `while.true` counts condition evaluations: the true arm is
  the number of iterations, the false arm is the number of exits, which is
  skipped-on-entry plus one exit per loop entry. The row on the guard is
  credited with the sum of both arms.
- The report accepts include and exclude path globs, because linked
  libraries resolve from the registry and otherwise swamp the summary.

## Alternatives

- Sum across all rows on a line: double counts lines with several
  instructions and nested control rows.
- Collapse consecutive rows with the same location, as cairo-coverage does:
  a heuristic that fails for two inlined copies separated by other
  instructions. It may still be needed for Rust sources, where one source
  span covers many instructions; that is a phase 3 question.
- Count functions by root node starts only: wrong for anything inlined.

## Consequences

- A call site line still has no row and stays absent (see roadmap).
- Rust-compiled packages need their own look at the per-location sum rule
  before phase 3 ships.
