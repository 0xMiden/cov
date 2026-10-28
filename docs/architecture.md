# Architecture

The map of miden-cov. It states the shape of the system, the vocabulary, the
rules the code implements, and the policies that are not visible in the code.
It deliberately contains no file paths, function names, or version numbers: the
code is the source of truth for those. If a sentence here and the code
disagree, find out which one is wrong before changing either.

Decisions and their alternatives are in [decisions/](decisions/README.md).
Planned work is the list of issues linked from the README.

## Pipeline

Coverage flows through four stages, each handing a plain artifact to the next.

1. **A package with debug info.** The assembler or the compiler produces a
   package whose debug-info section holds one row per source instruction and a
   tree of source nodes mirroring the MAST. This is the only input; the program
   is never modified and there is no coverage build.
2. **Collection.** The debugger engine steps the VM and resolves every cycle to
   source; the collector counts, from those records, which source nodes
   started, from which parent, and which operations ran. Counts are of
   attempts: a failing row counts, and so does the edge of a `call` whose
   callee is never found. A cycle whose source the engine could only guess
   is counted as a cycle and marks its package as touched, nothing more. A
   package with debug info that the execution passed through is reported
   whether or not anything in it was attributed. Invariant: a guessed source
   is never a hit; coverage may be lost, never invented. The one exception,
   accepted until the record names a guessed parent, is the edge from such a
   parent to a tracked child, which no count of the report reads.
3. **The coverage directory.** Each VM run ends as one run file holding its
   counts, and each package appears once with its forest and debug info. The
   directory is the only link between collecting and reporting.
4. **Reporting.** The reporter merges any number of run files, resolves the
   source files, applies the coverable rule, and writes LCOV for genhtml,
   octocov, and editor gutters, a terminal summary, and a JSON view for agents.

Collection and reporting never run in the same step. Runs from different
processes or machines merge by landing in the same directory.

## Crates

- **collector**: counts the per-cycle records the debugger engine produces and
  writes a run file. It also provides the program-executor implementation that
  test suites plug into the protocol's transaction executor; when coverage is
  asked for and can be attributed, that executor drives the engine's
  host-agnostic stepper with the host the test suite supplies, and otherwise
  hands the program to the processor's own executor and nothing changes. The
  engine is the only reader of the VM's continuation stream: the collector
  never interprets continuations itself.
- **cov**: reads a coverage directory, merges runs, resolves source files,
  computes line, function, and branch coverage, and writes the output formats.
  It never executes anything.
- **cli**: the `miden-cov` binary. `run` loads and links a package through the
  debugger engine, drives the engine's stepper with the collector attached, and
  writes a run file. `report` and `dump` are thin fronts over the cov crate.
  The debugger engine is used here for what it is good at: library lookup,
  package loading, event handlers, and the inputs file format users already
  know from the debugger.

## Glossary

Terms shared with the VM and the debugger. Most confusion in this codebase is
between the first two.

- **Exec node**: a node of the MAST forest, the thing the VM executes. Nodes
  are deduplicated by digest, so one exec node can correspond to several places
  in the source.
- **Source node**: a node of the debug-info tree that mirrors the exec nodes
  one source occurrence at a time. Two identical `if` blocks in different
  procedures share one exec node but have two source nodes, and a procedure
  that is inlined has a copy of its tree at every call site, whose rows carry
  the inline call that produced them. The VM reports the current source node
  while stepping, which is what makes attribution exact.
- **Row** (asm-op row): one entry of a source node's instruction table, one per
  MASM instruction as written. A row knows the index of the first VM operation
  it expanded to, how many operations it expanded to, its source location, its
  procedure path, and the instruction name. Control-flow instructions
  (`if.true`, `while.true`, `exec`, `call`, `begin`, ...) are rows with zero
  operations, attached to the source node they introduce.
- **Location**: a file plus a byte range. Line numbers are not stored; they
  require the source text.
- **Hit**: one observed attempt of a row or a start of a source node.
- **Edge start**: a start of a source node recorded together with the node that
  started it. A procedure entered dynamically or from another package has one
  source tree however many places call it, and the `call`s of one procedure
  within one inlined copy of their caller share one tree, so a node's own start
  count cannot tell those call sites apart; the edge from the calling node can.
- **Run**: one VM execution from entry to termination, recorded as one file.
- **Coverable**: a row that counts in the denominator. In a MASM package every
  row is coverable. Filtering is a Rust-source concern, planned with Rust
  support.
- **Coverage directory**: the on-disk exchange between collection and
  reporting.

## Coverable rule

This is the specification the collector and reporter implement together.

1. A row with one or more operations is hit each time its first operation
   is attempted. Its later operations are not counted, so a row that expands
   to five operations is hit once per execution, not five times.
2. A row with zero operations is coverable only when it belongs to a split
   node, where it stands for the condition and is hit once per evaluation.
   Zero-operation rows on join and loop nodes are structural duplicates of the
   procedure body and of the loop guard and are ignored.
3. Rows are aggregated by exact location. Rows with the same file and byte
   range are copies of one instruction, for example a procedure the assembler
   inlined at several call sites, and their counts are summed.
4. A location is credited to the line where it starts. Control rows span their
   whole block, and crediting every line in the span would mark unexecuted
   lines as covered.
5. A line's hit count is the maximum over the locations credited to it, never
   the sum, because one line can hold several instructions that each execute
   once per pass.
6. A function is hit each time the first coverable location that belongs to it
   is hit: the first row in its body, in execution order, whose procedure path
   is the function's own, stopping at the first conditional or loop. That
   counts inlined copies and bodies entered through an external node, whose
   root start is never seen, and skips rows of procedures inlined into the
   body. A function with no coverable row of its own counts the
   starts of its root, the entries of its inlined copies, found where a copy's
   inline call begins, and the edges into the tree a `call` node calls, when
   that tree's root belongs to one function across every package of the
   directory, records agreeing on file, line, and name being one function;
   a `call` into the function's own root is counted by the root's starts, not
   by its edge. The function's definition line is credited with that count,
   because LCOV consumers reject a function record without a line record.
   Invariant: the root start of a procedure entered through an external node
   is never recorded, or the edges and the starts would both count the entry.
7. Hits are counts. Any consumer that wants booleans truncates at output time.

## Branch semantics

Branches come from the MAST structure, not from pattern matching on
instructions.

- An `if.true` introduces a split node with two child source nodes, the true
  arm and the false arm. The assembler always emits both arms, an empty block
  standing in for a missing `else`. An arm's taken count is the number of edge
  starts from the split to that arm, not the arm node's total starts, which
  would include every other call of a procedure used as a bare arm.
- A `while.true` is lowered to a split node guarding a loop node: the split's
  true arm is the loop, its false arm is the empty block taken when the
  condition is false on entry. The branch counts condition evaluations the way
  gcov does: the true arm is the number of iterations, counted as edge starts
  from the loop to its body, the false arm is the number of exits, which is
  skipped-on-entry plus one exit per loop entry. Loop nodes themselves produce
  no branch record.
- A branch record anchors to the line where the split's own control row starts.
  A split never started reports both arms as not executed rather than as zero.

What is not a branch: `exec`, `call`, `dyncall`, and assertions.

## Coverage directory and run files

The directory is self-contained. The report step must never need the original
package files, because packages used by test suites are embedded in binaries
and never exist on disk.

- `packages/<digest>`: the MAST forest and the debug-info section of a package,
  written once per package digest by the first run that touches it. The forest
  is included because the reporter needs node kinds and procedure roots, and
  the original package may only exist inside a test binary. Identical content
  from concurrent writers is harmless. Nothing is ever removed: a directory
  is one build of the sources, and a report over packages of several builds
  mixes their functions, which is the user's to avoid by starting a fresh
  directory per build.
- `runs/<id>`: per package, the node starts, the edge starts, and the operation
  hits, plus run metadata (test name, outcome, cycles). Written on run
  completion, never appended to.

Both are versioned binary files using the VM's serialization traits. A version
bump in either format is a breaking change of the directory and the reporter
must refuse older versions with a message naming the version, never guess.

## Source resolution

Locations are byte ranges in files identified by path, as recorded at build
time. Those paths may be absolute from another machine or trimmed and relative.
The reporter resolves them against the current directory, a user-supplied
source root, and user-supplied prefix mappings, in that order. A file that
cannot be found is listed in the summary and skipped; it never aborts the
report.

## Test-suite activation

A test suite opts in by enabling the collector as a dependency and pointing the
protocol's transaction executor at the collector's program executor, which runs
the program through the engine's stepper with the suite's own host. The
collector steps and writes only when the coverage directory environment
variable is set; with the feature on and the variable unset, the program runs
through the processor's own executor and nothing changes. Each VM run, which
under the usual test runner means each test, writes its own run file named
after the test; no global state is shared between tests. Coverage never changes
a test's outcome: a run file that cannot be written is reported and the
execution's outputs are returned as usual.

## Version policy

Miden dependencies are published releases on `main`, the branch a release is
cut from. A change that needs an engine that is not released yet builds
against a pinned commit of the engine's own development branch, with the
decision record naming the commit's branch, and may land on `next` that way;
the pin goes before `next` is promoted to `main`, which the release checks
there refuse until it does. The workspace tracks one VM release: the one the
debugger engine pins,
since every crate here builds on the engine. If the protocol moves to a VM
release the debugger does not yet support, coverage of the protocol's tests
waits for the debugger to follow.

## Report scope

Every package with debug info that a run touched is reported, including
linked libraries whose sources resolve. Include and exclude path globs narrow the report; there is no
implicit notion of "the user's package", because for library coverage the
library is the target.

## Output policy

LCOV is the primary format and the only one the MVP writes. It is the common
input of genhtml, octocov, Codecov, Coveralls, and editor coverage gutters.
Line, function, and branch records are all emitted. Pull request comments, the
rendered HTML, and its storage as a workflow artifact all come from octocov;
miden-cov renders nothing itself. A JSON format sized for agents lists
uncovered regions grouped by function with per-test attribution.

## Documentation policy

- This file is the map: current shape, vocabulary, rules, policies. No facts
  that the code can confirm or refute.
- [decisions/](decisions/README.md) is the history: one record per decision,
  append-only, superseded records kept and linked.
- The GitHub issues linked from the README are the future: one per phase, with
  scope, acceptance criteria, and pending upstream changes in the body.
- Invariants are named here in one line each and enforced in code with a
  comment giving the reason. Formats and types are documented in rustdoc next
  to the code.
- A change that makes a design choice adds a decision record and updates the
  affected section here in the same PR.
