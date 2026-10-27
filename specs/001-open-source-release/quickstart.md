# Implementation handoff and validation quickstart

## Location and state

Worktree: `minos/001-open-source-release` in the bare-plus-worktrees layout.
Branch: `001-open-source-release`, rebased onto master at `27ba8f8` on 2026-10-26.
Original review baseline: `85eb785`.
Number allocation checked worktrees, all branches, existing specs and known
abandoned/reflog entries; no numbered feature existed, so max+1 was 001.
Master and its local wallet/transaction/report artifacts were left in place.

The `.specify` bootstrap is the installed skill's portable 0.4.2 snapshot, not a
claim that upstream Spec Kit was upgraded. Native entrypoint is the `speckit` skill.

## Resume

Read AGENTS.md, `.specify/memory/constitution.md`, `spec.md`, `research.md`, `plan.md`,
`data-model.md`, `contracts/interfaces.md`, and `tasks.md`. Resolve the storage,
provenance and legacy-identity design gates before their dependent implementation.
Implementation is authorized by the user's 2026-10-26 resume request. Baseline,
CI, wallet safety and signing work is recorded in tasks/research; the full release
is not ready. Consult those records for current verification and blockers.

```bash
# Run from the 001-open-source-release checkout root.
bash .specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
nix develop
```

## Validation commands for implementation

```bash
cargo test --workspace --locked
cargo build --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- --deny warnings
nix build .#minos .#minos-ui
nix flake check
```

The development workspace passes 146 Rust tests. Clean Nix CLI/portal package
checks pass all release-mode tests and the UI passes 14 Elm tests. Strict all-target
Clippy, Rust formatting, treefmt and flake checks pass. The two-stage publication
bundle replaces the oversized atomic CLI deployment; the atomic oversize case is
retained as a negative test. Process-boundary migration/restore rehearsal passes,
including exact restored contents and interrupted migration resume.

The TLS/Tokio/yanked dependency patches are applied. License-file audit and full
notice generation completed. Seven known transitive Wasm/ZIP findings and
unmaintained warnings remain explicitly documented, without suppressions, pending
maintainer acceptance or larger dependency migration. A separate sanitized Git
bundle has been prepared and scanned; original refs have not been rewritten.
Live preview, its faucet funding/node socket and the final maintainer decisions
remain open. No signing/submission or publication has occurred.

## Preview exercise

Prepare a disposable preview instance and testnet-only credentials separately.
Record supported publish/fund/evidence/pause/resume/modify/withdraw/final-output
flows and portal/UI results. No signing/submission is authorized by running this
quickstart or reading its tasks. Do not use the real mainnet wallet or recreate
ignored transaction files from master as part of implementation tests.

## Release review

Complete `checklists/release.md`, record version/support scope and migration notes,
and present artifacts/evidence to the maintainer. Repository publication, pushes,
release tags, production deployment and visibility changes remain outside this plan.
