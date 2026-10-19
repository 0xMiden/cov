#!/bin/bash
set -euo pipefail

base="${1:?usage: check-release-metadata.sh BASE_COMMIT}"

package_version() {
    local metadata="$1"
    local package="$2"

    printf '%s' "$metadata" |
        jq -er --arg package "$package" '.packages[] | select(.name == $package) | .version'
}

metadata_json="$(cargo metadata --locked --no-deps --format-version 1)"
worktree_root="$(mktemp -d)"
base_worktree="$worktree_root/base"
cleanup() {
    git worktree remove --force "$base_worktree" >/dev/null 2>&1 || true
    rmdir "$worktree_root" >/dev/null 2>&1 || true
}
trap cleanup EXIT
git worktree add --detach "$base_worktree" "$base" >/dev/null

# The base commit predates the Cargo workspace only before the first release.
if [[ -f "$base_worktree/Cargo.toml" ]]; then
    base_metadata_json="$(
        cargo metadata --locked --no-deps --format-version 1 --manifest-path "$base_worktree/Cargo.toml"
    )"

    # miden-cov inherits workspace.package.version.
    current_workspace="$(package_version "$metadata_json" miden-cov)"
    base_workspace="$(package_version "$base_metadata_json" miden-cov)"

    if [[ "$current_workspace" != "$base_workspace" ]] &&
        git show-ref --verify --quiet "refs/tags/v$current_workspace"; then
        echo "ERROR: v$current_workspace has already been released. Choose a newer workspace version." >&2
        exit 1
    fi
fi

scripts/check-package-release-plan.sh --skip-semver
