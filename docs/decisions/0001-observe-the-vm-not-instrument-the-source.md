# 0001: Collect coverage by observing the VM, not by instrumenting the source

Status: accepted (2026-10-19)

## Context

Coverage needs two things: which code ran, and the set of code that could have
run. The blockchain ecosystem has tried both ways of getting the first.
Foundry, cairo-coverage, Move, and the Solana sBPF tool observe the VM and map
executed locations back to source through compiler debug info. Hardhat's
solidity-coverage rewrites the source with probe calls and recompiles.

Miden packages already carry per-instruction source rows in a debug-info
section, emitted by the assembler for MASM and by the compiler for Rust, in
one schema.

## Decision

Run the unmodified package on the VM, record which source rows and source
nodes execute, and map them to source through the package's own debug info.
The set of coverable rows comes from enumerating the debug info, never from
the trace.

## Alternatives

- Source instrumentation: changes the program being measured, including its
  cycle count, which Miden tests assert on. Every tool that chose it reports
  the same regret.
- Host-side Rust coverage of the test process: measures the test harness, not
  what the VM executed.

## Consequences

- No coverage build profile; any package that kept its debug info is
  measurable (see 0009).
- Coverage accuracy is bounded by the debug info the assembler and compiler
  emit; gaps are fixed upstream, not worked around here.
