# Implementation Plan: Open-source release readiness

Branch: `001-open-source-release` | Date: 2026-10-12 | [Specification](spec.md)

## Summary

Harden wallet/signing and transaction correctness first, rebuild portal ingestion
around validated identity and recoverable projections, then complete portable
packaging, tests, docs and release evidence. Keep the existing successful withdrawal
fixture and fixes. Implementation was authorized on 2026-10-26 after rebasing onto
master at `27ba8f8`; the original plan was prepared without implementation authority.

## Technical context

- Rust 2021 workspace; Nix/Fenix toolchain and Cargo.lock are authoritative.
- Pallas 1.4.0 and uplc 1.1.24; preserve CBOR boundary between Pallas versions.
- Axum/async-graphql/Tokio; existing Sled + bundled DuckDB storage.
- Elm 0.19.1 UI, Nix-built static files; MAF generation uses reqwest/docx-rs.
- Initial packaging target: x86_64-linux. No new scalability SLA is invented.
- Correctness priorities: value conservation, witness preservation, authenticated
  provenance, replay equivalence and bounded operations over raw throughput.
- Tests: Rust units/integration/property cases as justified, mock HTTP/node/chain,
  persistent temp-store crash/replay tests, Elm tests, separate preview lifecycle.

## Constitution check

Design satisfies secret/witness preservation, evidence-based indexing, Nix-only
builds, consequential tests, and explicit workflow boundaries. The present baseline
violates several principles; those are remediation scope, not approved exceptions.
Recheck after the storage/identity designs and before declaring a release candidate.
The user's failure-recovery agreement applies during implementation; investigate
and report failures before proposing a different approach. No delegated agents,
mainnet signing/submission, production migration, or publishing is implied.

## Structure

Keep the three components. Proposed focused additions:

- `minos/src/commands/` for CLI adapters; `minos/src/workflows/` for testable policy.
- `minos/src/tx/` for shared typed balancing, fees, encoding and validation.
- `minos/src/maf/` for fetching, parsing and document generation.
- `minos-portal/src/` for explicit identity, provenance, projection and supervision
  boundaries; expose a library test surface if integration tests require it.
- `minos/tests/`, `minos-portal/tests/`, `minos-ui/tests/` for behavior regressions.
- `docs/` for support matrix, operations/migration, provenance and release procedure.
- `.github/workflows/` for clean-checkout release gates.

Avoid module churn before characterization tests. Exact extraction names may adapt
to source discovery without changing required behavior.

## Delivery sequence

1. Establish reproducible gates: include fixtures, capture regression baselines,
   fix strict lint causes, and prepare CI. This enables reliable subsequent work.
2. US1: secure wallet I/O and preserve signing envelopes. Use synthetic secrets;
   assert stdout/stderr and file permissions without reading real wallet material.
3. US2: shared asset-aware balancing and protocol calculations; fix final payouts;
   configure verified references and reward purposes; extend validation to each
   supported workflow. Preserve explicit failures for unsupported cases.
4. US3/US4 design gate: decide provenance rules, project/payout association, journal
   durability and schema migration using research.md. Do not guess these semantics.
5. Implement identity/provenance before projections; implement atomic/durable event
   processing, rollback/reset and reconciliation before enabling dynamic supervisors.
   Confirm pending registrations cannot mutate confirmed chain state.
6. US5: safe portal config, request/upstream limits, accurate API status and UI handling.
7. US6: finish workflow/MAF extraction, pagination, UI/MAF tests, docs/provenance,
   dependency/history inspection and release artifacts. Preserve public APIs where
   sound; version/document intentional compatibility changes.
8. Validate offline gates, then run a separately authorized preview lifecycle and
   review evidence. Publication remains a separate maintainer action.

## Validation strategy

Test requirements, not just implementation. Every review regression has a positive
case and a meaningful rejection/recovery case where relevant. Existing 28 tests
must remain meaningful; do not retain a passing test by weakening its invariant.
No arbitrary line-coverage percentage substitutes for the behavior matrix.

Required gates: locked workspace build/tests, rustfmt, strict Clippy, complete
Nix CLI/portal and UI builds, flake checks, Elm tests, dependency policy checks,
clean-checkout fixture availability and documentation walkthrough. A Nix package
test must execute integration tests rather than merely compiling the binary.
Property tests are useful for value conservation and signing round trips; limits
prevent adversarial CBOR/resource tests from exhausting CI.

Preview exercise: publish/register/fund/evidence/withdraw/pause/resume/modify and
final consumption for supported commands, followed by portal/UI verification.
Use disposable credentials and testnet-only funds under separate authorization.
Mock sequences cover deterministic rollback/crash cases unavailable on demand live.

## Migration and operational risks

New composite keys and event provenance may invalidate old guesses. Provide a
versioned migration or explicit backed-up rebuild preserving local registrations
and labels. Never silently drop another instance's state. Store failures must not
advance checkpoints. Reindex is an explicit operation, not a startup side effect.
New auth/bind defaults and corrected GraphQL status need upgrade documentation.
Contract provenance must distinguish base scripts, applied parameters and deployed
hashes; fixture data must remain public and independent of live availability.

## Completion evidence

Update tasks with test commands/results, clean artifact identities, support scope,
migration exercise results and unresolved limitations. A release candidate is not
complete while mandatory gates fail or security/correctness blockers are waived
only by calling the software experimental.

## Approved remediation follow-through (2026-10-26)

The user approved the bounded two-stage unsigned publication design after the
atomic 2-of-3 regression exceeded the protocol size limit. Keep the atomic builder
as a compatibility/negative regression; the CLI publishes an unsigned deployment
bundle: registry NFT plus treasury reference, followed by the vendor reference
funded from the first transaction's exact change output. The manifest binds body
hashes, ordered dependency outref and expected registry/script references. Validate
both stages independently, require first-stage confirmation before second-stage
submission, preserve all assets and expose reference readiness separately from
registry confirmation. No signing or submission is included.

Also approved: targeted dependency patches, locked online license-file audit and
migration rehearsal across process boundaries matching the offline CLI workflow.
Keep source/backup/registration/restore/resume assertions; no sleeps or retries
may conceal a lock failure. History rewrite and live preview remain separate.
