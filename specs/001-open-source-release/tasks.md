# Tasks: Open-source release readiness

Implementation authorized on 2026-10-26; baseline verification is recorded in research.
Unchecked tasks still require implementation and verification. No tasks authorize
signing, submission, production migration, publication or delegated agents.
On 2026-10-27 the maintainer accepted the documented scope and requested local
master integration, explicitly deferring T039 and final T040 release acceptance.
Paths are relative to this worktree; proposed files may be refined during design.
Use Nix. Explain failures and ask before implementing a different approach.

## Phase 1 — Reproducible baseline and gates

- [x] T001 [US6] Reproduce baseline build/tests/format/Clippy and record evidence in
  `specs/001-open-source-release/research.md`; preserve known failures (R13, R22).
- [x] T002 [US6] Include `minos/tests` and future portal/UI tests in
  `perSystem/packages.nix`; verify fixtures exist and integration tests actually run
  in clean package builds (FR-015, R12).
- [x] T003 [US6] Fix baseline lint defects in affected Rust modules and remove dead
  template code such as `flake/nixosModules/example.nix` or implement/document it;
  no blanket warning suppression (FR-015/016, R13/R18/R21).
- [x] T004 [US6] Add `.github/workflows/ci.yml` using pinned Nix/locked dependencies
  for formatting, strict lint, workspace tests and package/UI builds; subsequent
  tasks extend the suite (FR-015, R12/R13/R22).

## Phase 2 — US1: Secrets and signing

Dependencies: baseline captured in T001; release gate T004 must include new tests.

- [x] T005 [US1] Add synthetic wallet CLI/file tests in `minos/tests/wallet.rs` covering
  encrypted-output logs, explicit display, modes, existing files, symlinks and I/O
  errors; implement safe behavior in `wallet.rs` and `main.rs` (FR-001, R01).
- [x] T006 [US1] Add published Icarus derivation/address/signature vectors with source
  attribution in `minos/tests/fixtures/wallet/`; verify payment key/hash/network and
  account bounds without real wallet material (FR-001/002, R22).
- [x] T007 [US1] Recreate the indefinite-map witness-loss regression and extend
  `minos/tests/signing.rs` for tagged/indefinite sets, partial signatures, duplicate
  signing, trailing/malformed CBOR, datums/redeemers and byte-identical body hashing
  (FR-002, R02).
- [x] T008 [US1] Repair `minos/src/tx/sign.rs` using lossless typed/raw handling;
  reject unsupported shapes before writing; document compatibility and ensure
  existing witnesses/auxiliary data survive (FR-002, R02; depends T007).

## Phase 3 — US2: Correct builders and portable references

Dependencies: T002 and T008 before integrated signed-builder tests.

- [x] T009 [US2] Research pinned ledger/builder fee, reference-fee tiers, min-UTxO,
  collateral and witness-size rules; record supported protocol versions in
  `research.md` before replacing calculations (FR-004/006, R11).
- [x] T010 [US2] Add asset-conservation selection/change cases and implement full
  value preservation in `coin_select.rs`, `tx/` and workflow callers; cover native
  assets, insufficiency and malformed/overflowing quantities (FR-003/004, R10/R11).
- [x] T011 [US2] Add final-payout regressions to `minos/tests/withdrawal.rs` for
  residual ADA, unscheduled tokens and zero residual; fix output/datum decisions
  in `main.rs`/withdraw workflow and `tx/withdraw.rs` (FR-003, R09).
- [x] T012 [US2] Replace fixed minimum-output/fee/collateral assumptions with shared
  checked calculations in `minos/src/tx/`; rebalance when encoded size/change/fee
  changes; test large datums, token bundles, many witnesses and boundaries
  (FR-004, R11; depends T009/T010).
- [x] T013 [US2] Replace deployment-specific staking-reference dispatch with configured or
  verified discovery in `main.rs`/workflows and `references.rs`; retain hash/NFT and
  complete-response checks; test another deployment (FR-005, R14).
- [x] T014 [US2] Fix or explicitly reject multiple reward-script combinations in
  `tx/cbor_util.rs` and `tx/withdraw.rs`; test map preservation, ordering, redeemer
  indices, per-script budgets and multisig semantics (FR-005, R14).
- [x] T015 [US2] Add positive/negative builder integration cases for publish, fund,
  complete, withdraw, adjudicate, modify and mint in `minos/tests/`; account for
  script-based vendor authorization on every supported path (FR-006, R22).
- [x] T016 [US2] Extend shared validation to supported builders where applicable
  and document remaining ledger checks/protocol limits in `preflight.rs` and
  `docs/support-matrix.md`; retain treasury-reference and wrong-destination cases
  (FR-006, R11/R22; depends T012–T015).

## Phase 4 — US3/US4: Portal design and identity

Dependencies: design gates must be resolved before persistence/projection changes.

- [x] T017 [US3] Define per-event provenance rules from pinned contract/metadata
  source and actual transactions; record authorization versus confirmation and
  rejection policy in `contracts/interfaces.md` and `research.md` (FR-007, R03).
- [x] T018 [US3] Design output-lineage/project/milestone association, including
  legacy ambiguity, unsorted payouts, paused prefixes and lexical IDs; update
  `data-model.md` with concrete mapping/reindex policy (FR-008/009, R07/R08).
- [x] T019 [US4] Decide authoritative journal/store boundary and migration approach;
  specify checkpoint atomicity, crash recovery, rollback-to-origin, instance reset
  and retained local registrations in `data-model.md`/`research.md` (FR-010/011, R04/R05).
- [x] T020 [US3] Implement composite identity and schema versioning in portal
  `types.rs`/`db.rs`; add collisions-across-instances tests and migration fixtures
  (FR-008/011, R06/R07; depends T018/T019).
- [x] T021 [US3] Apply provenance validation consistently in portal `sync.rs`,
  `indexer.rs`, `parser.rs` and `schema.rs`; add unrelated-chain-metadata spoofing
  and pending-versus-confirmed registration regressions (FR-007, R03; depends T017/T020).
- [x] T022 [US3] Replace payout-count/prefix inference and paused-index/lexical-order
  assumptions with the designed mapping; expose unresolved cases, test multiple
  projects and non-prefix withdrawal/modify sequences (FR-009, R08; depends T018/T021).

## Phase 5 — US4: Durable indexing and operations

- [x] T023 [US4] Implement durable event application and checkpoint/dedup consistency
  in portal `db.rs`/`sync.rs`; fault-inject each write boundary and test concurrent
  multi-instance scanning (FR-010, R04/R06; depends T019–T021).
- [x] T024 [US4] Implement rollback/replay/reset of all projections and markers,
  including real event slots, origin and stale intersection recovery; compare
  state against clean replay and verify reset isolation (FR-010/011, R04/R05;
  depends T022/T023).
- [x] T025 [US4] Reconcile spent/finally consumed outputs and preserve full assets
  and provenance in portal `indexer.rs`/`sync.rs`; test snapshot/chain interleavings
  and distinguish failed queries from complete empty snapshots (FR-012, R16).
- [x] T026 [US4] Add supervisor/bootstrap handling for startup and dynamically
  registered instances in portal `main.rs`/`sync.rs`; test supported node-only,
  Koios-only and mixed setups, task failure/reconnect and shutdown (FR-012, R15).
- [x] T027 [US4] Implement and document restartable backup/migrate/reindex procedures
  in portal storage/CLI and `docs/operations.md`; test existing databases and
  preserve local registration data (FR-011, R05/R07; depends T020/T024–T026).

## Phase 6 — US5: API and UI operation

- [x] T028 [US5] Introduce explicit development auth bypass, configurable safe bind,
  validated network/backend/interval settings in portal `config.rs`/`main.rs` and
  example config; test secure defaults and migration errors (FR-013, R17).
- [x] T029 [US5] Add HTTP total/connect deadlines and appropriate size/concurrency/
  GraphQL complexity limits in `portal.rs`, portal `koios.rs`/`api.rs`/`schema.rs`,
  and MAF HTTP code; test stalls, error statuses and oversized/expensive requests
  (FR-013, R17).
- [x] T030 [US5] Coordinate confirmed/pending/unresolved/unavailable state across
  GraphQL schema, CLI parser and Elm decoders; test backward compatibility or
  explicit versioned migration (FR-014, R20; depends T022/T025).
- [x] T031 [US5] Add UI endpoint configuration and Elm tests in `minos-ui/tests/`
  covering error/loading/empty/status/asset decoding; wire Elm checks into Nix/CI
  (FR-014/015, R20/R22; depends T030).

## Phase 7 — US6: Maintainability and release preparation

- [x] T032 [US6] Extract CLI adapters and workflow services from `minos/src/main.rs`
  into focused modules while preserving characterized behavior and CLI contracts
  (FR-016, R18; depends US1/US2 tests).
- [x] T033 [US6] Consolidate shared HTTP, metadata and CBOR logic where semantics
  agree; replace swallowed persistence/parsing errors with typed failures and
  structured diagnostics in CLI/portal; test failed writes cannot report success
  (FR-016, R18; depends T021/T023/T029).
- [x] T034 [US6] Split `maf.rs` into fetching/parsing/rendering modules, implement
  pagination with explicit upstream-limit reporting, and add >100-result, malformed
  report, rate-limit and generated-document tests (FR-017, R19/R22).
- [x] T035 [US6] Add reproducible embedded-script provenance and verification
  instructions/assets in `docs/contracts.md`; verify source revision, compiler,
  build configuration and base/applied hashes (FR-018, R21).
- [x] T036 [US6] Add LICENSE/required attribution, CONTRIBUTING.md, SECURITY.md,
  changelog/release procedure, and accurate README/support/operations docs; remove
  or update stale CONTEXT.md and developer-specific assumptions; document GPG and
  UI deployment dependencies (FR-018, R21; depends T027/T028/T031/T035).
- [x] T037 [US6] Audit reachable history and release artifact contents for secrets
  and unintended private material using redacted findings; review locked dependency
  advisories/licenses and remediation policy; record limits/results without exposing
  secrets or rewriting history without authorization (FR-018, R21).

## Phase 8 — Integrated release gates

- [x] T038 [US6] Run clean-checkout workspace/Nix/UI/strict-lint/format tests with all
  fixtures present; inspect packaged binaries and runtime dependencies; attach
  command results and artifact identifiers to `checklists/release.md` (FR-015).
- [ ] T039 [US2/US4/US5] Prepare preview lifecycle fixtures/instructions, obtain
  separate authorization for disposable-key signing/submission, and exercise
  supported CLI → node → portal → UI workflows; record transaction/results evidence
  without secrets (FR-006/010/014; depends T016/T027/T031/T038).
- [ ] T040 [US6] Review requirement/finding coverage, migration rehearsal, dependency
  and history results; finalize support matrix/version decisions with maintainer,
  document accepted exclusions and evaluate release checklist. Do not publish or
  change repository visibility under this task (FR-018; depends T035–T039).

## Dependencies and completion rules

T001–T004 establish gates. US1 and US2 are independently testable slices. US3/US4
share T017–T020 design prerequisites; identity/provenance precede durable projections.
US5 config limits can proceed independently of schema-dependent UI changes. Refactors
follow characterization tests. No `[P]` marker or sequencing note authorizes subagents.

A checkbox means implemented and verified, not merely that a file exists. Record
failures and unresolved research explicitly; mandatory correctness/security checks
cannot be waived by changing a test expectation or release label.

## Requirement traceability

| Requirements | Tasks |
|---|---|
| FR-001 | T005–T006 |
| FR-002 | T006–T008 |
| FR-003 | T010–T011 |
| FR-004 | T009–T012 |
| FR-005 | T013–T014 |
| FR-006 | T015–T016, T039 |
| FR-007 | T017, T021 |
| FR-008 | T018, T020 |
| FR-009 | T018, T022 |
| FR-010 | T019, T023–T024, T039 |
| FR-011 | T019–T020, T024, T027 |
| FR-012 | T025–T026 |
| FR-013 | T028–T029 |
| FR-014 | T030–T031, T039 |
| FR-015 | T001–T004, T031, T038 |
| FR-016 | T003, T032–T033 |
| FR-017 | T034 |
| FR-018 | T035–T037, T040 |
