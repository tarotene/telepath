# Changelog

All notable changes to Telepath are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning 2.0.0](https://semver.org/).

Telepath uses **unified versioning** across all five workspace members
(`telepath-wire`, `telepath-macros`, `telepath-server`, `telepath-client`,
`host-pty-server`) plus two excluded crates (`tools/telepath`,
`examples/nrf52840-ping`). A single `vX.Y.Z` tag covers the whole repository.

<!-- release-plz-start -->

## [Unreleased]
## [0.2.3](https://github.com/tarotene/telepath/compare/v0.2.2..v0.2.3) — 2026-06-26

### Fixed

- **server,docs**: Correct ShimFn return type in README ([#228](https://github.com/tarotene/telepath/pull/228))


## [0.2.2](https://github.com/tarotene/telepath/compare/v0.2.1..v0.2.2) — 2026-05-31

### Fixed

- **ci,docs**: Align renovate MSRV constraint and fix stale README release claims ([#217](https://github.com/tarotene/telepath/pull/217))
- **ci**: Use cargo update --precise to avoid registry dep churn in bump-excluded ([#204](https://github.com/tarotene/telepath/pull/204))

## [0.2.1](https://github.com/tarotene/telepath/compare/v0.2.0..v0.2.1) — 2026-05-30

### Added

- **client**: Detect ambiguous cmd_id_by_name lookup ([#194](https://github.com/tarotene/telepath/pull/194))
- **macros,server**: Emit ResponseStatus::AppError for Result<T, AppErrorPayload> returns ([#193](https://github.com/tarotene/telepath/pull/193))

### Fixed

- **release**: Drop macOS Intel from binaries matrix + unify release name to v{{version}} ([#188](https://github.com/tarotene/telepath/pull/188))
- **ci**: Commit bump-excluded via GitHub API for verified signature ([#199](https://github.com/tarotene/telepath/pull/199))
- **ci**: Regenerate excluded-crate lockfiles in bump-excluded ([#201](https://github.com/tarotene/telepath/pull/201))

## [0.2.0](https://github.com/tarotene/telepath/compare/v0.1.0..v0.2.0) — 2026-05-29

### Breaking Changes

- **[BREAKING] wire, server, client**: rzCOBS upstream framing (Stage C2) — Host→Target uses COBS, Target→Host uses rzCOBS ([#176](https://github.com/tarotene/telepath/pull/176))
- **[BREAKING] wire, client**: Specify AppError payload format (Stage C4) ([#182](https://github.com/tarotene/telepath/pull/182))

### Added

- **client**: Typed `call::<Args, Ret>` wrapper (Stage C1) ([#174](https://github.com/tarotene/telepath/pull/174))
- **telemetry**: DWT-based framing/throughput instrumentation ([#178](https://github.com/tarotene/telepath/pull/178))
- **release**: Pre-built binary distribution pipeline for 4 platforms ([#184](https://github.com/tarotene/telepath/pull/184))

### Other

- **wire**: `impl From<postcard::Error> for WireError` ([#173](https://github.com/tarotene/telepath/pull/173))
- **deps**: Lock file maintenance; taiki-e/install-action update

## [0.1.0] — 2026-05-27

### Added

- **macros**: Detect cross-command cmd_id collisions at build time ([#98](https://github.com/tarotene/telepath/pull/98))

- **mcp**: Named-argument mapping for tuple-schema commands ([#59](https://github.com/tarotene/telepath/pull/59))

- **[BREAKING]** Role-based rename — server/client, flat layout, workspace consolidation ([#32](https://github.com/tarotene/telepath/pull/32))


### Other

- **deps**: Update wire protocol deps ([#152](https://github.com/tarotene/telepath/pull/152))

- **toolchain**: Declare MSRV 1.88 across workspace and excluded crates ([#105](https://github.com/tarotene/telepath/pull/105))

- Thread Issue links through README Limitations sections ([#85](https://github.com/tarotene/telepath/pull/85))

- Purge loopback zombies and fix mcp-server/shell docs post-#66 ([#81](https://github.com/tarotene/telepath/pull/81))


All entries below represent pre-0.1.0 history included here so that
release-plz does not retroactively generate a single oversized release entry.
Commits are grouped by type in reverse-chronological order within each group.

### Continuous Integration & Build

- chore(ci): split into 5 independent workflows with composite action and git-diff guard ([#110](https://github.com/tarotene/telepath/pull/110))
- chore(deps): introduce Renovate configuration ([#109](https://github.com/tarotene/telepath/pull/109))
- chore: track Cargo.lock for all manifest groups ([#108](https://github.com/tarotene/telepath/pull/108))
- chore(ci): speed up CI with just recipes, fmt job, and apt cache ([#104](https://github.com/tarotene/telepath/pull/104))
- chore(release): declare publish = false and repository for excluded crates ([#103](https://github.com/tarotene/telepath/pull/103))
- chore(githooks): delegate to just runner, add pre-push, document setup ([#29](https://github.com/tarotene/telepath/pull/29))
- chore: update repository URL to tarotene/telepath
- ci: extend clippy to workspace-excluded tools; fix rtt-only import gates ([#72](https://github.com/tarotene/telepath/pull/72))
- ci(host-emulator): assert ping → 0xDEADBEEF in smoke test ([#12](https://github.com/tarotene/telepath/pull/12))

### Toolchain & Release Infrastructure

- chore(toolchain): declare MSRV 1.88 across workspace and excluded crates ([#105](https://github.com/tarotene/telepath/pull/105))

### Documentation

- docs(readme): reflect open limitations for MSRV (#74) and prebuilt binaries (#52) ([#102](https://github.com/tarotene/telepath/pull/102))
- docs: sync #[resource] coverage across telepath-server and nrf52840-ping READMEs ([#100](https://github.com/tarotene/telepath/pull/100))
- docs(agents): recommend #[resource] over static Mutex<RefCell<Option<T>>> ([#97](https://github.com/tarotene/telepath/pull/97))
- docs(post-#92): add tools/telepath README and sweep stale references ([#95](https://github.com/tarotene/telepath/pull/95))
- docs: thread Issue links through README Limitations sections ([#85](https://github.com/tarotene/telepath/pull/85))
- docs(macros): add README for telepath-macros crate ([#83](https://github.com/tarotene/telepath/pull/83))
- docs: purge loopback zombies and fix mcp-server/shell docs post-#66 ([#81](https://github.com/tarotene/telepath/pull/81))
- docs(readme): rewrite lead + add Agent-ready by design section ([#34](https://github.com/tarotene/telepath/pull/34))
- docs(agents,cli): document #[command] macro accurately, fix chip example ([#31](https://github.com/tarotene/telepath/pull/31))
- docs: refresh READMEs for post-B2/B3/B4/B5 state ([#23](https://github.com/tarotene/telepath/pull/23))
- docs(hardware): mark nRF52840-DK real hardware path as experimental ([#10](https://github.com/tarotene/telepath/pull/10))

### Tests

- test(mcp): add serial PTY smoke test for telepath mcp subcommand ([#99](https://github.com/tarotene/telepath/pull/99))
- test(firmware): add registry_smoke covering linkme-collected commands() ([#24](https://github.com/tarotene/telepath/pull/24))

### Added

- feat(server): support Drop for ResourceRegistry entries ([#101](https://github.com/tarotene/telepath/pull/101))
- feat(macros): detect cross-command cmd_id collisions at build time ([#98](https://github.com/tarotene/telepath/pull/98))
- feat(server,macros): add #[resource] for type-safe peripheral injection ([#91](https://github.com/tarotene/telepath/pull/91))
- feat(shell): accept positional arguments without JSON array wrapping ([#89](https://github.com/tarotene/telepath/pull/89))
- feat(mcp): expose resources and prompts capabilities ([#61](https://github.com/tarotene/telepath/pull/61))
- feat(shell): add --exec flag; fix broken firmware-ping just recipe ([#62](https://github.com/tarotene/telepath/pull/62))
- feat(mcp): add RTT transport for flashed devices and fix stale docs ([#65](https://github.com/tarotene/telepath/pull/65))
- feat(client): SchemaCache::clear and TelepathClient::rediscover ([#60](https://github.com/tarotene/telepath/pull/60))
- feat(mcp): named-argument mapping for tuple-schema commands ([#59](https://github.com/tarotene/telepath/pull/59))
- feat(example/loopback,nrf): CPU-only demo commands — add, crc32, echo ([#58](https://github.com/tarotene/telepath/pull/58))
- feat(example/nrf): on-chip sensor and ID commands — TEMP, HWRNG, FICR, SAADC ([#57](https://github.com/tarotene/telepath/pull/57))
- feat(shell): auto-reset target on RTT attach failure ([#56](https://github.com/tarotene/telepath/pull/56))
- feat(example/nrf): GPIO commands — LED control and button read ([#49](https://github.com/tarotene/telepath/pull/49))
- feat(shell): discovery-driven REPL + RTT ch0 log redirection ([#51](https://github.com/tarotene/telepath/pull/51))
- feat(mcp): telepath-mcp-server ([#35](https://github.com/tarotene/telepath/pull/35))
- feat(wire,firmware,host): offset-based paging for discovery responses ([#28](https://github.com/tarotene/telepath/pull/28))
- feat(wire,firmware,host): add postcard-schema fingerprints to DiscoveryEntry ([#27](https://github.com/tarotene/telepath/pull/27))
- feat(host): implement TelepathClient::discover to populate SchemaCache ([#21](https://github.com/tarotene/telepath/pull/21))
- feat(firmware): implement TelepathServer::handle_discovery (CDP responder) ([#20](https://github.com/tarotene/telepath/pull/20))
- feat(firmware): register CommandMetadata via linkme distributed slice ([#16](https://github.com/tarotene/telepath/pull/16))
- feat(macros): implement #[command] body with shim and CommandMetadata generation ([#15](https://github.com/tarotene/telepath/pull/15))
- feat(wire): add FNV-1a 16-bit cmd_id derivation ([#14](https://github.com/tarotene/telepath/pull/14))
- feat(emulator): add in-process host+firmware loopback example ([#4](https://github.com/tarotene/telepath/pull/4))
- feat: implement wire framing, RPC server/client, and CLI tool ([#1](https://github.com/tarotene/telepath/pull/1))

### Changed

- refactor(cli): unify wire path through TelepathClient via RttTransport ([#6](https://github.com/tarotene/telepath/pull/6))
- refactor: remove in-process loopback; symmetric build-time transport selection ([#66](https://github.com/tarotene/telepath/pull/66))
- Consolidate telepath-shell and telepath-mcp-server into unified `tools/telepath` CLI ([#92](https://github.com/tarotene/telepath/pull/92))

### Performance

- perf(client): replace per-byte read in call_raw with 256-byte chunked read ([#70](https://github.com/tarotene/telepath/pull/70))

### Fixed

- fix(just): release chip after flash so RTT attach succeeds on first try ([#73](https://github.com/tarotene/telepath/pull/73))
- fix(client): store Duration in RttTransport for per-call read timeout ([#71](https://github.com/tarotene/telepath/pull/71))
- fix(rtt): drain RPC ch1 before discovery; add attach timing instrumentation ([#68](https://github.com/tarotene/telepath/pull/68))
- fix(hardware): pin RTT control block to 0x20000000 for reliable telepath-cli attach ([#11](https://github.com/tarotene/telepath/pull/11))
- fix(wire): correct rzCOBS upstream framing claim in crate-level doc ([#8](https://github.com/tarotene/telepath/pull/8))

### BREAKING CHANGES

- **feat!: role-based rename — server/client, flat layout, workspace consolidation ([#32](https://github.com/tarotene/telepath/pull/32))** — Renamed `firmware/` → `telepath-server/`, `host/` → `telepath-client/`; flattened the crate layout; all import paths changed.

<!-- release-plz-end -->

[Unreleased]: https://github.com/tarotene/telepath/compare/aca2da0...HEAD
