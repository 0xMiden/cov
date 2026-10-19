# miden-cov

Code coverage for Miden VM programs: Miden Assembly (MASM) and Rust compiled with the
Miden compiler.

Coverage is collected by observing VM execution and is mapped back to source files through
the debug info carried in Miden packages. Reports are produced in the LCOV format, so they
work with the usual coverage tooling (`genhtml`, IDE gutters, CI coverage services).

## Status

Early development. The MVP measures a standalone MASM program: run it, merge the runs, and
render LCOV with line, function, and branch coverage. See the [roadmap](docs/roadmap.md)
for what comes next.

## Example report

The LCOV output of a small MASM program, rendered by `genhtml`. The run took the `if.true`
arm, so the `else` body shows zero hits and the branch markers show one arm untaken.

![Coverage of a MASM program, rendered by genhtml from the LCOV output](docs/images/fixture-report.png)

## Usage

Coverage needs a package (`.masp`) that kept its debug-info section. The assembler and the
Miden compiler keep it under the `dev` profile and strip it under `release` unless the
project sets `debug = true` for that profile in `miden-project.toml`. There is no separate
coverage build.

```sh
# Execute the package once per input set. Flags mirror `miden-debug`.
miden-cov run program.masp --inputs inputs-a.toml --name a
miden-cov run program.masp --inputs inputs-b.toml --name b

# Merge all runs, print a summary table, and write an LCOV trace.
# Linked libraries are reported too when their sources resolve; narrow the
# report with --include and --exclude path globs.
miden-cov report --lcov lcov.info --include 'src/**'

# Render HTML from the trace. --prefix keeps the page tree shallow.
genhtml --branch-coverage --prefix "$PWD" --output-directory html lcov.info
```

Runs go to the coverage directory, `target/miden-cov` by default or `MIDEN_COV_DIR` when
set; `--cov-dir` overrides both. Each run is a separate file, so runs from different
processes or machines merge by being placed in the same directory. `report` resolves source
paths as stored in the debug info; use `--source-root <dir>` or
`--path-prefix-map OLD=NEW` when the sources live elsewhere. `miden-cov dump <run-file>`
prints the raw hits of one run as JSON.

## Roadmap

- [x] Phase 1: pure MASM programs; `run`, `report`, `dump`; LCOV with line, function, and
      branch coverage
- [ ] Phase 2: in-test collection for protocol, VM, and user contract tests
- [ ] Phase 3: Rust programs compiled with the Miden compiler
- [ ] Phase 4: agent-facing JSON output and a skill for test-writing agents
- [ ] Phase 5: GitHub integration with PR comments and a hosted HTML diff view

See the full [roadmap](docs/roadmap.md) for scope and acceptance criteria.

## Building and testing

The toolchain is pinned in `rust-toolchain.toml`. Tests run with
[cargo-nextest](https://nexte.st/).

```sh
make build        # build the workspace in release mode
make test         # run the test suite
make lint         # clippy, rustfmt check and cargo-shear
make fix          # apply clippy and rustfmt fixes
make doc          # build and check the documentation
make help         # list all targets
```

Formatting uses a pinned nightly rustfmt; install it with `make install-rustfmt`.

## Documentation

- [Architecture](docs/architecture.md)
- [Roadmap](docs/roadmap.md)
- [Design decisions](docs/decisions/)
- [Release process](docs/release-process.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

This project is [MIT licensed](LICENSE.md).
