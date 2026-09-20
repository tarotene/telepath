# Contributing

See [README](README.md) for what this repository is and how to set up a
development environment.

## Issues

Judging question: Does it change what a `#[command]` author writes, or how a
host discovers/calls commands over a transport?

Accepted:
- feat(client): reconnect with backoff on serial transport
- feat(mcp): surface command doc-comments as MCP tool descriptions

Rejected:
- feat(server): built-in task scheduler for firmware apps (firmware
  application logic beyond reference examples is out of scope, see
  [README § Scope](README.md#scope))
- feat(examples): add STM32H7 board support crate (board support beyond
  reference examples is out of scope, see [README § Scope](README.md#scope))

## Pull requests

- Follow [Conventional Commits](https://www.conventionalcommits.org/) message
  format; the `commit-msg` hook enforces this via `cocogitto` (see
  [README § Git hooks setup](README.md#git-hooks-setup)).
- Create a feature branch before making any code change.
- Reference the corresponding GitHub Issue in the PR description.
- Run `just ci` locally before opening a PR (see
  [README § CI / Quality gates](README.md#ci--quality-gates)).
- PRs that touch `telepath-wire`, `telepath-macros`, `telepath-server`,
  `telepath-client`, `tools/telepath`, or `examples/nrf52840-ping` should be
  smoke-tested with `just firmware-ping` against a connected nRF52840-DK
  before requesting review, with the result recorded in the PR's test plan.
- Merges are squash-only, and all review threads must be resolved before
  merge (enforced by the repository's branch protection rulesets).

## Expectations

This is currently a single-maintainer project. Response times to Issues and
PRs may vary; Renovate-authored dependency PRs are reviewed on a weekly
cadence.
