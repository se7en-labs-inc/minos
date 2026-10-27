# Final acceptance review

Review updated 2026-10-27. T001–T038 have implementation and verification
evidence. The maintainer accepted candidate 0.1.0/x86_64-linux and the documented
dependency exclusions, then authorized local master integration with sanitized
history and private backup. Integrated preview and final release acceptance
(T039/T040) are explicitly deferred. This is not release certification.

## Requirement and finding coverage

| Coverage | Evidence reviewed | Remaining acceptance work |
|---|---|---|
| FR-001–002: wallet and signing | Published derivation/signature vectors, private exclusive files, explicit mnemonic display, lossless multi-witness CBOR tests | None in offline scope |
| FR-003–006: transaction construction | Full-value selection/change, dynamic fee/minimum/collateral checks, pinned-contract positive and negative lifecycle tests, bounded two-stage publication | Confirm supported workflows against the preview node in T039 |
| FR-007–009: portal provenance and identity | Witness-bound authorship, validated script/registry provenance, composite instance/project identity, non-prefix payout lineage and unresolved ambiguity tests | Exercise independent instances and payout observations in T039 |
| FR-010–012: recovery and ingestion | Whole-block journal crash boundaries, rollback/clean-replay equality, dynamic supervision/reconnect, failed snapshot handling, process-boundary migration/restore | Record live restart/catch-up and migration; controlled rollback remains explicitly fixture evidence |
| FR-013–014: API/UI operations | Authenticated writes, loopback defaults, bounded requests/backends, API-v2 statuses, exact references, configured UI endpoint and 14 Elm tests | Record the packaged UI against the same live portal in T039 |
| FR-015–017: reproducibility and maintenance | Clean committed-candidate packages/flake checks, 146 Rust tests, strict lint/format, extracted workflows, pagination above 100 results and document tests | Refresh candidate/artifact IDs after final acceptance records |
| FR-018: release materials | Contract provenance, license inventory/notices, operations/support/security/contribution docs, verified sanitized history bundle | Maintainer scope/dependency/history decisions in T040 |

The migration rehearsal preserves the source and backup, local registrations and
restored contents; it clears only rebuildable projections. Each maintenance
operation crosses a process boundary. Interrupted migration refuses ordinary
startup until explicitly resumed. No sleep or retry hides a database lock error.

## Maintainer disposition

- Version/platform: candidate 0.1.0, x86_64-linux, with the transaction/network/
  backend limits in `docs/support-matrix.md`. Mainnet integration is not claimed
  from a preview rehearsal.
- Dependency findings: accept the documented Linux/input scope. Seven transitive vulnerability entries and
  three unmaintained dependencies remain visible in `dependency-review.md`.
  No blanket advisory suppression and no zero-advisory audit claim are proposed.
- History: use the verified sanitized history for local master integration,
  retaining the original history privately. Preserve approved author/email attribution.
- Publication: remains outside this task. No remote push, tag, visibility change
  or release publication is authorized by this review.

The maintainer accepted the proposed target/input scope and sanitized history,
then requested that the feature become master. Preserve original history and
modified operational notes privately; leave all local transaction, mnemonic,
encrypted wallet and report files untouched. No remote destination is configured.
The integrated rehearsal and final release decision remain open, deferred by
the maintainer; neither is recorded as passing.

## T039 execution status

The maintainer requested work on the last two tasks. This authorizes rehearsal
signing/submission using the already prepared fresh disposable preview keys;
operational keys and mainnet transactions are outside that authorization.

Completed preparation and observations:

- Fresh isolated portal database/configuration, preview magic 2, loopback listener,
  and explicit fresh-key allowlist; development authentication bypass is disabled.
- The packaged CLI queried the packaged portal successfully and returned
  `{"apiVersion":2,"instances":[]}`. No instance or confirmation was fabricated.
- The fresh fee address has no confirmed UTxOs in the HTTP backend query.
- No preview node socket is configured or available in the checked conventional
  runtime locations. A socket path and fresh faucet funding were requested.
- The portal logged a closed upstream Koios connection while polling the tip.
  This is retained as a failed upstream observation, not live readiness evidence.

No rehearsal transaction has been signed or submitted. No live node lifecycle,
UI observation or preview confirmation is claimed. Continue the sequence in
`docs/preview-rehearsal.md` once the preview node and faucet funding are supplied;
then attach public hashes, slots and API/UI observations before closing T039.

## Completion rule

Close T039 only after the required integrated rehearsal has recorded evidence.
Close T040 only after reviewing that evidence and recording actual maintainer
decisions. Refresh the sanitized candidate, exact committed-source package/flake
checks, privacy scan and source-only `release-results.json` after final records.
