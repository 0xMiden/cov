# -------------------------------------------------------------------------------------------------
# Makefile
# -------------------------------------------------------------------------------------------------

.DEFAULT_GOAL := help

# -- help -----------------------------------------------------------------------------------------
.PHONY: help
help:
	@printf "\nTargets:\n\n"
	@awk 'BEGIN {FS = ":.*##"; OFS = ""} /^[a-zA-Z0-9_.-]+:.*?##/ { printf "  \033[36m%-24s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\nExamples:\n"
	@printf "  make test EXPR=\"'my::mod::some_test'\" # Run a specific test\n\n"


# -- environment toggles --------------------------------------------------------------------------
BACKTRACE                := RUST_BACKTRACE=1
# Stable and beta rustfmt ignore the unstable settings even with RUSTC_BOOTSTRAP.
RUSTFMT_TOOLCHAIN        := nightly-2026-10-09

# -- feature configuration ------------------------------------------------------------------------
ALL_FEATURES             := --all-features

# Target triple used when producing release artifacts. Defaults to the host's triple.
BUILD_TARGET             ?= $(shell rustc -vV | grep host | awk '{print $$2}')

# -- linting --------------------------------------------------------------------------------------

# Deny warnings locally so `make clippy`/`make lint` fail on the same warnings as CI,
# which runs `make clippy` with RUSTFLAGS=-D warnings (see .github/workflows/lint.yml).
DENY_WARNINGS := RUSTFLAGS="$(RUSTFLAGS) -D warnings"

# The clippy lint set is defined once, in the `xclippy` cargo alias in
# .cargo/config.toml, and extracted here so the check and fix paths cannot
# drift apart. cargo-fixit (a faster drop-in for `cargo clippy --fix`) cannot
# take lint flags as trailing arguments, so the fix path passes them via
# RUSTFLAGS instead.
CLIPPY_LINT_FLAGS := $(shell sed -n '/^xclippy = \[/,/^]/p' .cargo/config.toml | grep -oE '"-[WD][^"]+"' | tr -d '"' | tr '\n' ' ')

.PHONY: clippy
clippy: ## Runs Clippy with configs (alias for xclippy)
	$(DENY_WARNINGS) cargo +stable xclippy


.PHONY: xclippy
xclippy: ## Runs Clippy with custom lint config from .cargo/config.toml
	$(DENY_WARNINGS) cargo +stable xclippy


.PHONY: fix
fix: xclippy-fix format ## Applies automatic lint and format fixes

.PHONY: xclippy-fix
xclippy-fix: ## Applies clippy lint fixes via cargo-fixit (a faster `cargo clippy --fix`)
	@if ! command -v cargo-fixit >/dev/null 2>&1; then \
		echo "cargo-fixit is not installed; skipping clippy lint fixes." >&2; \
		echo "It is a faster drop-in replacement for 'cargo clippy --fix'." >&2; \
		echo "Install it with:  cargo install cargo-fixit --locked" >&2; \
	else \
		RUSTFLAGS="$(CLIPPY_LINT_FLAGS)" cargo +stable fixit --clippy \
			--allow-dirty --allow-staged \
			--workspace --all-targets --all-features; \
	fi


.PHONY: install-rustfmt
install-rustfmt: ## Installs the pinned nightly formatter
	rustup toolchain install $(RUSTFMT_TOOLCHAIN) --profile minimal --component rustfmt --no-self-update

.PHONY: format
format: ## Runs Format using nightly toolchain
	cargo +$(RUSTFMT_TOOLCHAIN) fmt --all


.PHONY: format-check
format-check: ## Runs Format using nightly toolchain but only in check mode
	cargo +$(RUSTFMT_TOOLCHAIN) fmt --all --check

.PHONY: shear
shear: ## Runs cargo-shear to find unused or misplaced dependencies
	cargo shear

.PHONY: lint
lint: xclippy format-check shear ## Runs all lint checks without modifying files

# --- docs ----------------------------------------------------------------------------------------

.PHONY: doc
doc: ## Generates & checks documentation for workspace crates only
	rm -rf "$${CARGO_TARGET_DIR:-target}/doc"
	RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS="--enable-index-page -Zunstable-options -D warnings" cargo +stable doc ${ALL_FEATURES} --keep-going --release --no-deps

# -- testing --------------------------------------------------------------------------------------

NEXTEST_PROFILE ?= default
# Filter expression/selector passed through to nextest, e.g.:
#   -E 'not test(#*proptest)'   or   'my::module::test_name'
EXPR            ?=
# Extra args to nextest (e.g., --no-run)
EXTRA           ?=

.PHONY: test
test: ## Run the workspace test suite; accepts EXPR= and EXTRA= for nextest
	$(BACKTRACE) cargo nextest run --workspace --profile $(NEXTEST_PROFILE) $(ALL_FEATURES) $(EXTRA) $(EXPR)

.PHONY: test-build
test-build: ## Build the test binaries for the workspace (no run)
	$(MAKE) test EXTRA="--no-run"

# --- checking ------------------------------------------------------------------------------------

.PHONY: check
check: ## Checks all targets and features for errors without code generation
	cargo check --workspace --all-targets ${ALL_FEATURES}

# --- building ------------------------------------------------------------------------------------

.PHONY: build
build: ## Builds with default parameters
	cargo build --workspace --release

# --- executable ----------------------------------------------------------------------------------

.PHONY: exec-dist
exec-dist: ## Builds the miden-cov CLI for $(BUILD_TARGET) with --locked (for release artifact uploads)
	cargo build -p miden-cov-cli --bin miden-cov --release --target $(BUILD_TARGET) --locked
# NOTE: the resulting binary is stored in target/<$(BUILD_TARGET)>/release/
