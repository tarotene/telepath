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

## Excluded crates: why bump-excluded is required

`tools/telepath` and `examples/nrf52840-ping` are listed under `exclude` in
the root `Cargo.toml` and are therefore invisible to release-plz. Their
`version` field is not touched by the automated version bump.

The `just bump-excluded X.Y.Z` recipe (`Justfile`) rewrites the `version`
line in both files with `sed`. Run it on the release PR branch before merging:

```
just bump-excluded 0.1.1
git add tools/telepath/Cargo.toml examples/nrf52840-ping/Cargo.toml
git commit -m "chore(release): bump excluded crates to 0.1.1"
git push
```

## Reference

- [`release-plz.toml`](../release-plz.toml) — workspace configuration
- [`.github/workflows/release-plz.yml`](../.github/workflows/release-plz.yml) — workflow file
- [release-plz docs](https://release-plz.dev/docs)
- Tracking issue: [#30](https://github.com/tarotene/telepath/issues/30)
