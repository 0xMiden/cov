# 0013: Map, records, roadmap; the code is the source of truth

Status: accepted (2026-10-19)

## Context

Design documents rot when they repeat what the code says, because nobody
updates two places. They also rot when documents that change at different
rates are mixed into one file. The project is built largely by agents whose
only context is what the repository tells them, so the reasoning behind
decisions must be findable without re-deriving it.

## Decision

Three kinds of document under `docs/`:

- The architecture map: current shape, vocabulary, rules the code implements,
  and policies invisible in the code. Rewritten in place. It contains no file
  paths, function names, or version numbers, and no sentence that the code can
  confirm or refute.
- Decision records: one per decision, append-only, superseded ones kept and
  linked.
- The roadmap: phases, acceptance criteria, pending upstream changes.

Invariants are named in the map in one line and enforced in code with a
comment giving the reason. Types and formats are documented in rustdoc. A
change that makes a design choice adds a record and updates the map in the
same PR. When a document and the code disagree, determine which is wrong
before changing either.

## Alternatives

- One design document: mixes rates of change and rots.
- Per-crate design documents: duplicates rustdoc.
- A wiki outside the repository: not versioned with the code, invisible to
  agents.

## Consequences

- Per-task scratch documents in a worktree are distilled into these before a
  PR merges and are never committed.
- Reviews check that new invariants named in a PR appear in the map.
