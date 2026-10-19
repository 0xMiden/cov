# miden-cov

Code coverage for Miden VM programs: Miden Assembly (MASM) and Rust compiled with the
Miden compiler.

Coverage is collected by observing VM execution and is mapped back to source files through
the debug info carried in Miden packages. Reports are produced in the LCOV format, so they
work with the usual coverage tooling (`genhtml`, IDE gutters, CI coverage services).

## Status

Early development. The crates and the `miden-cov` command-line tool are scaffolds; nothing
is functional yet.

## Workspace

| Crate | Path | Description |
| --- | --- | --- |
| `miden-cov-collector` | [`crates/collector`](crates/collector) | Collects coverage while the VM executes a program. |
| `miden-cov` | [`crates/cov`](crates/cov) | Maps coverage to source and renders reports. |
| `miden-cov-cli` | [`crates/cli`](crates/cli) | The `miden-cov` command-line tool. |

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

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

This project is [MIT licensed](LICENSE.md).
