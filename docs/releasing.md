# Release Runbook

Telepath releases are automated via release-plz. This document covers
non-standard scenarios: retriggering, version overrides, and recovery.

For the normal release cycle, see
[AGENTS.md § How releases work](../AGENTS.md#how-releases-work).

## Retriggering the release workflow

If the `release-plz-pr` or `release-plz-release` job fails or is skipped,
retrigger it via `workflow_dispatch`:

```
gh workflow run release-plz.yml --repo tarotene/telepath --ref main
```

Or from the GitHub UI: Actions → Release-plz → Run workflow.

## Overriding the next version

release-plz calculates the next version from Conventional Commits since the
last tag. To override it (e.g. force a minor bump for a patch-only commit set):

```
# Preview what the current config would produce
just release-preview

# Override a specific package version before the release PR opens
release-plz set-version X.Y.Z --package telepath-wire
```

After running `set-version`, commit and push; release-plz will pick up the
pinned version on the next run.

## Recovering from a duplicate or stale release PR

release-plz closes old release PRs automatically when it creates a new one.
If multiple stale PRs accumulate (e.g. after repeated workflow failures),
close them manually:

```
gh pr list --repo tarotene/telepath --label release --state open
gh pr close <NUMBER> --repo tarotene/telepath
```

Then retrigger the workflow to get a fresh PR.

## Recovering from a bad release

A published GitHub Release cannot be "unpublished" in a way that removes it
from the Releases page, but it can be deleted if it has not been announced.

```
# Delete a release and its tag
gh release delete telepath-wire-vX.Y.Z --repo tarotene/telepath --cleanup-tag --yes
```

After deletion, fix the root cause in a follow-up commit. release-plz will
produce a new release PR for the corrected version on the next push to `main`.

**Do not re-use a deleted tag.** Increment the patch version instead
(e.g. `v0.1.0` deleted → next release is `v0.1.1`).

## Excluded crates: bump-excluded requirement

`tools/telepath` and `examples/nrf52840-ping` are listed under `exclude` in
the root `Cargo.toml` and are therefore invisible to release-plz. Their
`version` field is not touched by the automated version bump.

**As of #170, this step is automated.** The `release-plz-pr` job reads the
bumped version from the workspace `Cargo.toml` on the release PR branch,
runs `just bump-excluded`, and pushes a commit
`chore(release): bump excluded crates to X.Y.Z` onto that branch automatically.

### Recovery: if the automatic bump fails

If the automatic commit was not pushed (e.g. the `release-plz` action output
did not contain a branch name, or the push was rejected), run the step manually:

```
git fetch origin <release-pr-branch>
git checkout <release-pr-branch>
just bump-excluded X.Y.Z
git add tools/telepath/Cargo.toml tools/telepath/Cargo.lock \
        examples/nrf52840-ping/Cargo.toml examples/nrf52840-ping/Cargo.lock
git commit -m "chore(release): bump excluded crates to X.Y.Z"
git push
```

`just bump-excluded` rewrites both `Cargo.toml` files **and** regenerates their
`Cargo.lock` files (via `cargo update -p <name> --precise <version>` in each
directory — only the local package entry is updated, registry dependencies are
left untouched). The `git add` above captures all four changed files.

Replace `X.Y.Z` with the version from `[workspace.package].version` in the
root `Cargo.toml` on the release PR branch.

## Trusted Publishing

The release workflow uses **Trusted Publishing (OIDC)** — no long-lived `CARGO_REGISTRY_TOKEN`
secret is stored in the repository. GitHub Actions exchanges a short-lived OIDC token
(30-minute lifetime) with crates.io at publish time, with no rotation or monitoring required.

**Setup requirement for new crates**: Before a crate can be published for the first time via
the release workflow, a Trusted Publishing entry must exist on crates.io:

1. Go to <https://crates.io/settings/tokens> → "Trusted Publishing" → "Add publisher".
2. Set: repository `tarotene/telepath`, workflow filename `release-plz.yml`.
3. Optional: restrict to a specific environment name for extra isolation.

See [release-plz quickstart § Trusted Publishing](https://release-plz.dev/docs/github/quickstart)
and the [crates.io Trusted Publishing docs](https://crates.io/docs/trusted-publishing).

### Initial bootstrap (historical — v0.2.0 only)

The first publish of any crate on crates.io cannot use Trusted Publishing
([crates.io limitation](https://crates.io/docs/trusted-publishing)). For v0.2.0, a
short-lived API token (scopes: `publish-new` only, 7-day expiry) was used for the one-time
manual bootstrap, then immediately revoked:

```bash
# Dependency-ordered initial publish
cargo publish -p telepath-wire && sleep 30
cargo publish -p telepath-macros && sleep 30
cargo publish -p telepath-server && sleep 30
cargo publish -p telepath-client && sleep 30
(cd tools/telepath && cargo publish)
```

Once all five crates are live on crates.io, configure Trusted Publishing for each one.
For each of `telepath-wire`, `telepath-macros`, `telepath-server`, `telepath-client`, `telepath`:

1. Open `https://crates.io/crates/<CRATE>/settings` (signed in as the publisher account).
2. Scroll to **Trusted Publishers** → **Add a new publisher**.
3. Fill in:
   - Repository owner: `tarotene`
   - Repository name: `telepath`
   - Workflow filename: `release-plz.yml`
   - Environment name: *(leave blank)*
4. Save.

After all five crates have a Trusted Publishing entry, revoke the short-lived API token at
`https://crates.io/settings/tokens`.

Then create the matching git tag and GitHub Release so the repository state aligns with
what was published:

```bash
git tag -a v0.2.0 -m "Release v0.2.0 (initial crates.io publish)"
git push origin v0.2.0
gh release create v0.2.0 \
  --title "v0.2.0" \
  --notes-file <(awk '/^## \[0\.2\.0\]/,/^## \[/' CHANGELOG.md | sed '$d')
```

Trusted Publishing handles all subsequent releases automatically — no rotation or monitoring needed.

### tools/telepath (excluded crate)

`tools/telepath` is excluded from the root workspace and is not managed by release-plz.
The `release-plz-release` job publishes it in a separate step using
[`rust-lang/crates-io-auth-action`](https://github.com/rust-lang/crates-io-auth-action).

**Why a separate action?** release-plz performs its own OIDC token exchange internally
and does not expose that token to subsequent workflow steps. release-plz's quickstart
says "don't use `crates-io-auth-action`" — this applies only to the crates managed
by `release-plz release` itself. For crates published outside release-plz's scope,
`crates-io-auth-action` is the correct mechanism and must have a matching Trusted
Publishing entry on crates.io (same owner/repo/workflow as the workspace crates).

## Reference

- [`release-plz.toml`](../release-plz.toml) — workspace configuration
- [`.github/workflows/release-plz.yml`](../.github/workflows/release-plz.yml) — workflow file
- [release-plz docs](https://release-plz.dev/docs)
- Tracking issue: [#172](https://github.com/tarotene/telepath/issues/172)
