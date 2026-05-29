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

## Rotating CARGO_REGISTRY_TOKEN

The token issued for the initial crates.io publish expires **90 days** after issue.
The `token-expiry-check` workflow opens an issue when the token is 60+ days old.

**Name convention**: `telepath-release-plz-YYYYQn` (e.g. `telepath-release-plz-2026Q3`).

Rotation steps (zero-downtime — old token stays valid until you revoke it):

1. Issue a new token at <https://crates.io/me> → Account Settings → API Tokens → New Token.
   Scopes: `publish-new` + `publish-update` only. Expiration: 90 days.
   Copy the `cio_…` value immediately (shown only once).

2. Update the secret and the issue-date variable:
   ```
   echo -n "<new-token>" | gh secret set CARGO_REGISTRY_TOKEN \
     --repo tarotene/telepath --body -
   gh variable set CARGO_TOKEN_ISSUED_AT --body "$(date +%F)" \
     --repo tarotene/telepath
   ```

3. Confirm authentication with a dry-run workflow trigger:
   ```
   gh workflow run release-plz.yml --repo tarotene/telepath --ref main
   ```
   Expected: both jobs complete (or skip if no pending release).

4. Revoke the old token at <https://crates.io/me> → Account Settings → API Tokens → Revoke.

### Expired token recovery (401 Unauthorized)

If the token already expired and release-plz is failing with 401:

```
# 1. Issue new token (Step 1 above)
# 2. Overwrite the secret
echo -n "<new-token>" | gh secret set CARGO_REGISTRY_TOKEN \
  --repo tarotene/telepath --body -
# 3. Update issued-at variable
gh variable set CARGO_TOKEN_ISSUED_AT --body "$(date +%F)" \
  --repo tarotene/telepath
# 4. Retrigger the failed workflow
gh workflow run release-plz.yml --repo tarotene/telepath --ref main
```

## Initial crates.io publish (one-time choreography)

The first publish must be performed manually in dependency order because
release-plz cannot compute diffs against a non-existent registry entry.

**Prerequisites**: `CARGO_REGISTRY_TOKEN` secret set, `RELEASE_PLZ_ENABLED` variable unset or `false`.

```bash
# From workspace root — each publish waits for crates.io index (~30 s) before continuing
cargo publish -p telepath-wire && sleep 30
cargo publish -p telepath-macros && sleep 30
cargo publish -p telepath-server && sleep 30
cargo publish -p telepath-client && sleep 30
(cd tools/telepath && cargo publish)
```

After publishing, enable automation and create the GitHub Release:

```bash
# Tag and release
git tag -a v0.2.0 -m "Release v0.2.0 (initial crates.io publish)"
git push origin v0.2.0
gh release create v0.2.0 \
  --title "v0.2.0" \
  --notes-file <(awk '/^## \[0\.2\.0\]/,/^## \[/' CHANGELOG.md | sed '$d')

# Re-enable release-plz automation
gh variable set RELEASE_PLZ_ENABLED --body true --repo tarotene/telepath
gh variable set CARGO_TOKEN_ISSUED_AT --body "$(date +%F)" --repo tarotene/telepath
```

Verify all five crates are live:

```bash
for crate in telepath-wire telepath-macros telepath-server telepath-client telepath; do
  curl -sf "https://crates.io/api/v1/crates/$crate/0.2.0" >/dev/null \
    && echo "$crate ✓" || echo "$crate ✗"
done
```

## Reference

- [`release-plz.toml`](../release-plz.toml) — workspace configuration
- [`.github/workflows/release-plz.yml`](../.github/workflows/release-plz.yml) — workflow file
- [`.github/workflows/token-expiry-check.yml`](../.github/workflows/token-expiry-check.yml) — token age monitor
- [release-plz docs](https://release-plz.dev/docs)
- Tracking issue: [#172](https://github.com/tarotene/telepath/issues/172)
