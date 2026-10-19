# 0014: Agent feedback is a JSON of uncovered regions with per-test attribution

Status: accepted (2026-10-19)

## Context

One goal of the project is fast feedback for agents that write tests for Miden
programs. An agent needs to know what to reach next and which existing test
is the closest starting point, in a form that fits a context window. LCOV and
HTML are built for humans and coverage services.

## Decision

The report step can emit a JSON document listing uncovered regions grouped by
function, each with a short source excerpt and location, plus for each file
the tests that touched it. Hit counts are kept per run so the attribution is
exact. A skill or equivalent instruction set wraps the CLI for agents. This is
phase 4 of the roadmap; the run file records the test name from the start so
no data is lost before then.

## Alternatives

- A library API for asserting coverage inside tests: useful later, but not
  the feedback loop that was asked for.
- An MCP server: a wrapper over the same JSON; can be added without a design
  change.

## Consequences

- Run files carry test names from the MVP on.
- The JSON schema is documented in rustdoc and versioned like the run file.
