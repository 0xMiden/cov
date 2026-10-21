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
check-no-std: ## Checks that the collector builds without its std feature, warnings denied
	RUSTFLAGS="-D warnings" cargo check -p miden-cov-collector --no-default-features

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
