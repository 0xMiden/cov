# -------------------------------------------------------------------------------------------------
# Makefile
# -------------------------------------------------------------------------------------------------

.DEFAULT_GOAL := help

# -- help -----------------------------------------------------------------------------------------
.PHONY: help
help:
	@printf "\nTargets:\n\n"
	@awk 'BEGIN {FS = ":.*##"; OFS = ""} /^[a-zA-Z0-9_.-]+:.*?##/ { printf "  \033[36m%-26s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\nExamples:\n"
	@printf "  make test EXPR=\"'my::mod::some_test'\" # Run a specific test\n"
	@printf "  make release-set-version ARGS=\"0.2.0 --dry-run\" # Preview a version bump\n\n"


# -- environment toggles --------------------------------------------------------------------------
BACKTRACE                := RUST_BACKTRACE=1
# Stable and beta rustfmt ignore the unstable settings even with RUSTC_BOOTSTRAP.
RUSTFMT_TOOLCHAIN        := nightly-2026-10-09

# -- feature configuration ------------------------------------------------------------------------
ALL_FEATURES             := --all-features

# -- linting --------------------------------------------------------------------------------------

.PHONY: clippy
clippy: ## Runs Clippy with the default lint set, warnings denied
	cargo clippy --workspace --all-targets -- -D warnings

.PHONY: fix
fix: ## Applies automatic clippy and format fixes
	cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged
	$(MAKE) format

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

.PHONY: check-no-std
check-no-std: ## Lints the collector without its std feature, warnings denied
	cargo clippy -p miden-cov-collector --no-default-features -- -D warnings

.PHONY: lint
lint: format-check clippy shear check-no-std ## Runs all lint checks without modifying files

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

# -- coverage -------------------------------------------------------------------------------------

# The Rust code coverage of this repository's own test suite (not the coverage of Miden
# programs that miden-cov measures). Integration tests, their fixtures, `tests/common` and the
# example projects are excluded from the measured set. Needs cargo-llvm-cov and the llvm-tools
# rustup component.
.PHONY: coverage
coverage: ## Measure the Rust code coverage of the test suite into rust-coverage.xml (Cobertura)
	$(BACKTRACE) cargo llvm-cov nextest --workspace --profile $(NEXTEST_PROFILE) $(ALL_FEATURES) --ignore-filename-regex '/(tests|examples)/' --cobertura --output-path rust-coverage.xml $(EXTRA) $(EXPR)

# The coverage of the Miden code of the example projects, measured the way a user measures their
# contracts: the example's tests run with the coverage directory set, then `miden-cov report`,
# built from this checkout, renders it. The directory is emptied first since one directory must
# hold one build of the sources. BASELINE= (an LCOV of the base branch) and DIFF= (a unified diff
# of the change) add the comparison; PAGE_URL= is where the page is published, for the comment.
MIDEN_COV_DIR_OUT        := target/miden-cov
MIDEN_COVERAGE_OUT       := target/miden-coverage
MIDEN_COVERAGE_TITLE     := Miden code coverage
MIDEN_COV                 = cargo run --locked --quiet -p miden-cov-cli -- report --cov-dir $(MIDEN_COV_DIR_OUT) --title "$(MIDEN_COVERAGE_TITLE)" $(if $(BASELINE),--baseline $(BASELINE)) $(if $(DIFF),--diff $(DIFF))

.PHONY: miden-coverage
miden-coverage: ## Measure the coverage of the examples' Miden code into target/miden-coverage (LCOV and HTML); accepts BASELINE= and DIFF=
	rm -rf $(MIDEN_COV_DIR_OUT)
	MIDEN_COV_DIR=$(CURDIR)/$(MIDEN_COV_DIR_OUT) $(BACKTRACE) cargo nextest run --profile $(NEXTEST_PROFILE) -p miden-cov-example-wallet $(EXTRA)
	$(MIDEN_COV) --lcov $(MIDEN_COVERAGE_OUT)/lcov.info --html $(MIDEN_COVERAGE_OUT)/miden-coverage.html
	@grep -q '^DA:[0-9]*,[1-9]' $(MIDEN_COVERAGE_OUT)/lcov.info || { echo 'error: the Miden coverage report has no hits' >&2; exit 1; }

.PHONY: miden-coverage-comment
miden-coverage-comment: ## Render the pull request comment of the last `make miden-coverage`; accepts BASELINE=, DIFF= and PAGE_URL=
	$(MIDEN_COV) --no-summary --markdown $(MIDEN_COVERAGE_OUT)/coverage-comment.md $(if $(PAGE_URL),--page-url $(PAGE_URL))

# --- checking ------------------------------------------------------------------------------------

.PHONY: check
check: ## Checks all targets and features for errors without code generation
	cargo check --workspace --all-targets ${ALL_FEATURES}

# --- building ------------------------------------------------------------------------------------

.PHONY: build
build: ## Builds with default parameters
	cargo build --workspace --release

# --- release -------------------------------------------------------------------------------------

# Local, read-only or repository-local commands only. `publish` and `finalize` are deliberately
# absent: they cross irreversible boundaries and belong to the release workflow, where an approval
# gate and a short-lived token stand between them and production. A convenient local shortcut for
# them would be a way to skip that by accident. See docs/release-process.md.

# Keep in sync with the install step in .github/workflows/release.yml.
RELEASE_TOOL_VERSION     := 0.1.0
RELEASE_TOOL             := GITHUB_REPOSITORY=0xMiden/cov release-tool
# Arguments passed through to `release-tool`, e.g. ARGS="lint" or ARGS="--unit cov 0.2.0".
ARGS                     ?=

.PHONY: install-release-tool
install-release-tool: ## Installs the pinned release-tool if it is not on PATH
	@command -v release-tool >/dev/null 2>&1 || \
		cargo install --locked miden-release-tool --version $(RELEASE_TOOL_VERSION) --bin release-tool

.PHONY: release
release: install-release-tool ## Invokes a release-tool command given in ARGS, e.g. ARGS="reconcile --plan plan.json"
	$(RELEASE_TOOL) $(ARGS)

.PHONY: release-lint
release-lint: install-release-tool ## Checks the release configuration and the candidate in .release/
	$(RELEASE_TOOL) lint $(ARGS)

.PHONY: release-package-order
release-package-order: install-release-tool ## Prints the crates.io publication order
	$(RELEASE_TOOL) package-order $(ARGS)

.PHONY: release-verify-closure
release-verify-closure: install-release-tool ## Packages the crates and builds a consumer resolved only from a registry
	$(RELEASE_TOOL) verify-closure $(ARGS)

.PHONY: release-set-version
release-set-version: install-release-tool ## Moves the release version; ARGS="0.2.0" (next minor if empty)
	$(RELEASE_TOOL) set-version --unit cov $(ARGS)

.PHONY: release-changelog-prompt
release-changelog-prompt: install-release-tool ## Emits a prompt for the changelog entries; ARGS="--version 0.2.0"
	$(RELEASE_TOOL) changelog-prompt cov $(ARGS)
