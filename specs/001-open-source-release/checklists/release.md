# Release acceptance checklist

Status: offline implementation and clean committed-candidate package/flake gates
pass. The maintainer accepted the target/input scope and local master integration;
live preview and final release acceptance are explicitly deferred.
Final output identifiers are recorded in the source-only `release-results.json`;
that evidence file is excluded from package inputs to avoid self-referential hashes.
Research records the prior failures and their approved corrections.
The final requirement/decision review and current live-rehearsal observations are
in [acceptance-review.md](../acceptance-review.md).

- [x] US1: no implicit secret output; private/no-clobber wallet files; published vectors pass.
- [x] US1: signing preserves body/witnesses for the supported CBOR compatibility matrix.
- [x] US2: every supported builder conserves assets and validates fee/min-UTxO/collateral.
- [x] US2: final payouts/residual value, alternate deployment references and reward-script cases pass.
- [x] US2: historical withdrawal positive/negative regressions remain meaningful.
- [x] US3: metadata spoofing rejected; instance/project isolation and associations proven.
- [x] US4: rollback/origin/restart/crash results equal clean replay; reset is scoped.
- [x] US4: dynamic bootstrap, stale/spent output reconciliation and native assets verified.
- [x] US4: existing-store migration/reindex rehearsed with backup and local data preserved.
- [x] US5: safe auth/bind/network defaults and request/upstream bounds tested.
- [x] US5: CLI/GraphQL/Elm status compatibility and deployment endpoint tests pass.
- [x] US6: MAF pagination/parsing/document tests pass; incomplete sources are reported.
- [x] US6: clean committed-candidate Rust, Nix, Elm, formatting and strict lint gates pass.
- [x] US6: test fixtures and runtime dependencies are present in correct release artifacts.
- [x] US6: embedded script provenance/reproduction and attribution documented.
- [x] US6: LICENSE, CONTRIBUTING, SECURITY, support matrix, operations and release docs ready.
- [x] US6: history/artifact secret review and dependency/license/advisory review recorded.
- [ ] Integrated preview lifecycle completed under separate authorization, with evidence.
- [x] Maintainer accepts candidate 0.1.0/x86_64-linux and the scoped dependency findings.
- [x] Maintainer chooses sanitized history and authorizes local master integration with private backup.
- [ ] No mandatory release acceptance gate remains unresolved.

## Verified behavior and commands

- `cargo test --workspace --locked --no-fail-fast`: all146 Rust tests pass.
- `cargo clippy --workspace --all-targets --locked -- --deny warnings`: passes.
- `nix fmt -- --fail-on-change`: passes; imported upstream notice bytes are retained.
- `nix build .#minos .#minos-ui --no-link`: passes all release-mode Rust and14 Elm tests.
- `nix flake check`: passes, including packaged strict Clippy and formatting.
- Both installed binaries start with development-shell `LD_LIBRARY_PATH` removed.
  Installed notice checksums pass against `licenses/SHA256SUMS`.
- Two-stage 2-of-3 publication passes size/protocol checks, preserves native assets,
  rejects missing dependencies/insufficient funds/collateral and retains the
  oversized atomic negative regression. Reference readiness is separate from
  registry confirmation and follows rollback.
- Cross-process migration/restore verifies exact source/backup/restore contents,
  retained local registrations, derived-state clearing and interrupted resume.
- All reachable sanitized-history commits pass personal-home-path search;
  `CONTEXT.md` is excluded from every commit. Gitleaks reports no secrets.
  author/email sets and the captured final source tree are preserved. Original
  history is backed up privately for authorized local master integration, and
  local operational files are preserved.
- Locked license-file audit:383 crates/111 license texts; full Rust/Elm/native
  notices installed. TLS/Tokio/yanked findings patched. Seven existing transitive
  Wasm/ZIP findings and unmaintained warnings have scoped maintainer acceptance;
  unfiltered cargo-audit still exits nonzero and no suppressions were added.
- Prepared live sequence: `docs/preview-rehearsal.md`. Fresh unfunded preview-only
  roles exist in private state; disposable-preview signing is authorized, while
  socket/faucet funding and live evidence are deferred. No real signing/submission
  or publication has occurred.

Publication is outside this implementation scope and is not authorized by these
checks or by acceptance of the history candidate.
