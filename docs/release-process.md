# Release Process

How `miden-cov` is released. The process and the tool are the Miden compiler's;
run `release-tool --help` for the commands. The `make release*` targets wrap
the tool and install the pinned version on first use.

Three things are true throughout and explain most of the procedure:

- **Publication to crates.io cannot be undone.** A version can never be reused.
  Everything reversible happens before anything irreversible, and the approval
  gate sits exactly on that boundary.
- **A release is resumable.** Every attempt reconciles against live state first,
  so a run that dies partway through is re-run and does only what is missing.
- **Versions are chosen by a person.** The tooling validates a version; it never
  picks one.

---

## 1. Branches

| Branch | Role |
| --- | --- |
| `next` | Development. All ordinary work merges here. |
| `main` | Releases. Only ever released from here. |

A release starts by promoting `next` into `main`, and the release candidate is
branched from and merged into **`main`**. The workflow enforces this: the commit
it releases must be `main`'s tip *and* the most recent commit to have changed
`.release/release.toml`.

---

## 2. What this repository releases

One unit, `cov`, declared in [`.release/config.toml`](../.release/config.toml).

| Unit | Crates, in publication order | Version lives in | Tag | Changelog | Assets |
| --- | --- | --- | --- | --- | --- |
| `cov` | `miden-cov-collector`, `miden-cov`, `miden-cov-cli` | the workspace version | `v{version}` | `CHANGELOG.md` | `miden-cov-<target>.tar.gz` for `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` |

Every workspace package must be classified in the config; `make release-lint`
fails on one that is not. A stable release claims the repository's "Latest
release" slot.

---

## 3. Prerequisites

One-time setup, requiring repository-admin and crates.io-owner access.

| What | Detail |
| --- | --- |
| crates.io Trusted Publisher, per crate | Repository `0xMiden/cov`, workflow `release.yml`, environment `release`. A crate that does not exist yet cannot have a publisher: bootstrap it once with a short-lived, narrowly scoped token, then switch it. |
| Trusted-Publishing-only mode | Enable per crate after its publisher is verified working. No `CARGO_REGISTRY_TOKEN` secret. |
| Immutable releases | Enabled for the repository. |
| Environment `release` | Required reviewers; self-review disabled; deployment restricted to protected `main`. |
| Tag ruleset for `v*` | Restrict deletions and updates only. Do not restrict creations: a bypass would have to cover every workflow, since `GITHUB_TOKEN` acts as the repository-wide Actions app. |
| Required status check | `release / gate` |

---

## 4. Production release

### Phase A: prepare the candidate

Entirely in pull requests; nothing touches crates.io or creates a tag.

1. **Promote `next` into `main`** with a pull request, merged once CI passes.
2. **Freeze `main`** until the release completes. Anything landing after the
   candidate invalidates it ([R3](#r3-subject-is-not-mains-tip)).
3. **Branch from `main`:**
   `git switch --detach origin/main && git switch -c release/v0.2.0`.
4. **Set the version.** Never hand-edit it:
   ```bash
   make release-set-version ARGS="0.2.0"     # empty ARGS: next minor; add --dry-run to review
   ```
   This edits the workspace version, the workspace dependency requirements,
   `Cargo.lock`, and `.release/release.toml`. To replace a version already
   bumped in this candidate (e.g. `0.2.0-rc.1` after `0.2.0`), add `--force`.
5. **Write the changelog:**
   ```bash
   make release-changelog-prompt ARGS="--version 0.2.0"
   ```
   This emits a *prompt* listing the commits since the last `v*` tag; it never
   writes entries. Write the `## [0.2.0]` section of `CHANGELOG.md` under the
   headings the prompt names, and review it as prose.
6. **Lint:** `make release-lint`. Expect
   `release lint: 3 packages classified, no findings`.
7. **Open the candidate pull request into `main`** with the version, lockfile,
   changelog, and `.release/release.toml`. `release / gate` runs at the full
   tier, including the package closure ([R4](#r4-package-closure-verification-fails)).
8. **Merge** so the candidate is `main`'s tip and the last commit to have
   touched `.release/release.toml`.

### The pull-request gate

`release-ci.yml` runs on every pull request and exposes one required check,
`release / gate`. It calls `release-verify.yml`, the same workflow production
calls, so pull requests and releases run identical checks.

| Job | What it does |
| --- | --- |
| `impact` | Classifies the diff. Touching `.release/`, `Makefile`, `.github/workflows/`, `Cargo.lock`, or any `Cargo.toml` escalates to the full tier. |
| `release lint` | Lints the configuration and checks the publication order is derivable. |
| `package closure` | Packages every crate and builds a consumer resolved only from a registry. Full tier only. |
| `workflow policy` | Asserts the release workflow's invariants; runs `actionlint`. |
| `gate` | Fails if any required job failed *or was unexpectedly skipped*. |

### Phase B: plan

**B1. Dispatch the release workflow from `main`.** It has no inputs; the scope
comes from `.release/release.toml` alone.

```bash
gh workflow run release.yml --ref main
gh run watch "$(gh run list --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
```

**B2. Review the intent** printed by the `plan` job's **Generate the intent**
step (also the `release-intent` artifact): version, prerelease flag, crates,
tag, and the "latest" decision. If anything is unexpected, cancel the run;
nothing irreversible has happened.

### Phase C: build and stage

Automated and reversible: `verify` runs `release-verify.yml` at the full tier,
`seal` packages the crates and pins their digests into the plan, the `build`
matrix builds, smoke-tests, and archives `miden-cov` on Linux and macOS,
`attest artifacts` records build provenance, and `stage drafts` creates the
draft release and uploads and reads back every asset. No tag exists and nothing
is published. Attestations are the one permanent side effect; they are harmless.

### Phase D: approve and publish

**D1. Approve the deployment. This is the point of no return.** The `publish`
job waits in the `release` environment. Check the intent, the draft and its
assets, and any warnings, then **Review deployments** → **release** →
**Approve and deploy**. The approver must be a required reviewer other than the
person who dispatched the run. The approval also covers Phase E.

**D2. Publication runs:** create the tag, obtain a short-lived Trusted
Publishing token, reconcile against crates.io, publish what is absent, and
verify it from the registry. The `release-journal` artifact records every
decision.

### Phase E: finalize

**E1.** `finalize` verifies the draft (tag points at the released commit, the
draft carries this run's plan, every asset hashes to what was staged, nothing
unplanned is attached) and publishes it. A published release is immutable.

**E2.** Confirm the release is no longer a draft, the crates are on crates.io,
and `cargo install miden-cov-cli` resolves the new version. Unfreeze `main`.

---

## 5. Local rehearsal

Exercises packaging and the publish path against a local registry that proxies
crates.io. Nothing reaches crates.io or GitHub, and no tag is created. Needs an
unpublished version, so run it on a candidate branch after A4.

```bash
# Terminal 1: the local registry. Put the source replacement it prints in
# $CARGO_HOME/config.toml.
make release ARGS="fake-registry --port 8732 --cache-dir /tmp/rehearsal-cache"

# Terminal 2, from the candidate workspace:
make release-verify-closure
make release ARGS="plan --subject $(git rev-parse HEAD) --output intent.json"
make release ARGS="seal --intent intent.json --output plan.json --cache-dir /tmp/rehearsal-cache"
make release ARGS="publish --plan plan.json --rehearsal-index sparse+http://127.0.0.1:8732/"
```

## 6. Hosted rehearsal

A production run stopped at the approval gate. Follow §4 through Phase C and
**do not approve** at D1; clean up with [§8.1](#81-discarding-drafts). It
exercises the real GitHub path (drafts, uploads, attestations) without tags or
crates.

## 7. Prereleases

Give a version with a prerelease identifier, e.g. `0.2.0-rc.1`, at A4 and follow
§4 unchanged. The GitHub release is marked prerelease and never becomes
"Latest", and a default requirement such as `miden-cov = "0.1"` never matches it.

---

## 8. Recovery

### 8.1 Discarding drafts

Cancel the run, then delete the draft in the **Releases** UI, or download the
sealed plan from the run's artifacts and:

```bash
GITHUB_TOKEN=$(gh auth token) make release ARGS="discard --plan plan.json"
```

`discard` deletes only still-draft releases.

### 8.2 Resuming a partial publish

Re-dispatch the workflow from `main` (B1). Crates already on the registry at the
planned version and digest are skipped. **Do not change versions to resume.**
To preview what a resume would do:

```bash
GITHUB_TOKEN=$(gh auth token) make release ARGS="reconcile --plan plan.json"
```

### 8.3 What cannot be undone

A published crate version, a created tag, a finalized release, and a recorded
attestation are all permanent. The recovery is always the same: **abandon that
version and release a new one.**

### R1: `set-version` reports disagreeing versions

A crate's version was hand-edited. Set them all to the same version, re-run
`make release-set-version`, and confirm with `make release-lint`.

### R2: `release lint` findings

| Finding | Remedy |
| --- | --- |
| `package '…' is not classified` | A new workspace member. Add it to `.release/config.toml`. |
| publishable in its manifest but classified otherwise | Make the manifest and the config agree. |
| `active [patch] entry` | Remove it; the published crate would resolve without the patched behaviour. |

### R3: Subject is not `main`'s tip

Something merged to `main` after the candidate. Open a fresh candidate from the
current `main` and re-dispatch.

### R4: Package closure verification fails

A packaged crate is missing something: a file outside its `include`, or a
dependency without a version requirement. Reproduce with
`make release-verify-closure` and read the build error.

### R5: 403 from crates.io during publication

The named crate's Trusted Publisher is missing or misconfigured (§3). Fix it and
re-dispatch; do not change versions.

### R6: An asset does not match what was uploaded

Do not publish. Cancel, discard the draft (§8.1), and re-dispatch. If it recurs,
treat it as an integrity incident.

### R7: A tag already exists at a different commit

That version is burnt. Discard the draft and release a new version.

### R8: A planned version conflicts or is yanked

Open a new candidate with a new version.

### R9: The publishing token expired

Tokens live 30 minutes. Re-dispatch; reconciliation resumes from what is
already published.

### R10: The release declaration was changed in a different commit

`.release/release.toml` was not last modified by the commit being released.
Confirm `set-version` ran and was committed, then open a fresh candidate.

### R11: An artifact is routed nowhere

The build matrix in `release.yml` and the `assets` globs in
`.release/config.toml` disagree. Fix one of them and re-dispatch.

### R12: Finalization reports an unplanned asset

Do not publish. Find where it came from; if it was attached by hand, remove it
and re-dispatch.

### R13: `crate already exists` during a local rehearsal

The rehearsed version is already on crates.io. Rehearse an unpublished version.
