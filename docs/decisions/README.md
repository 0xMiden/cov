# Decision records

One file per decision: context, decision, alternatives and why they lost,
consequences, status. Append-only. A reversed decision gets a new record; the
old one is marked superseded and links forward. Never delete a record.

| # | Decision | Status |
|---|----------|--------|
| [0001](0001-observe-the-vm-not-instrument-the-source.md) | Collect coverage by observing the VM, not by instrumenting the source | accepted |
| [0002](0002-step-api-until-tracer-is-exported.md) | Observe through the processor's step API until the tracer trait is exported | accepted |
| [0003](0003-cli-uses-the-debugger-engine.md) | The CLI uses the debugger engine for loading, linking, and inputs | accepted |
| [0004](0004-collector-stays-engine-free.md) | The collector crate never depends on the debugger engine | accepted |
| [0005](0005-vm-version-policy.md) | Published releases only; the workspace follows the debugger's VM pin | accepted |
| [0006](0006-coverage-unit-and-coverable-rule.md) | Rows and source nodes are the unit; hit counts, not booleans | accepted |
| [0007](0007-branches-from-mast-structure.md) | Branch coverage from split and loop nodes, in the MVP | accepted |
| [0008](0008-coverage-directory.md) | Two-stage workflow through a self-contained coverage directory | accepted |
| [0009](0009-input-is-a-package-with-debug-info.md) | Input is a package that kept its debug info; no coverage build | accepted |
| [0010](0010-lcov-primary-output.md) | LCOV is the primary output; octocov for PR comments | accepted |
| [0011](0011-test-suite-activation.md) | Test suites activate coverage through a feature plus an environment variable | accepted |
| [0012](0012-repository-organization.md) | Crates under `crates/`; VM-style Makefile, CI, and release; compiler-style Cargo conventions | accepted, release part superseded by 0015 |
| [0013](0013-documentation-policy.md) | Map, records, roadmap; the code is the source of truth | accepted |
| [0014](0014-agent-facing-output.md) | Agent feedback is a JSON of uncovered regions with per-test attribution | accepted |
| [0015](0015-release-process-from-the-compiler.md) | Release process from the compiler, not the VM | accepted |
| [0016](0016-aggregate-by-location.md) | Aggregate hits by location; count functions by their first instruction | accepted |
| [0017](0017-record-node-starts-per-parent-edge.md) | Record node starts per parent edge | accepted |
