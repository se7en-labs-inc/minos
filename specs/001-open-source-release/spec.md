# Feature Specification: Open-source release readiness

**Feature Branch**: `001-open-source-release`
**Created**: 2026-10-12
**Status**: Offline gates pass; scope accepted and master integration authorized; live preview/final release acceptance deferred (2026-10-27)
**Baseline**: `27ba8f8` after rebasing onto master on 2026-10-26;
original review baseline `85eb7858b83c81cfb596a40a0f6ed577b6015a1f`
**Input**: Capture the complete release-readiness review in Spec Kit in a new worktree.

## Scope and release definition

Prepare Minos CLI, portal, and Elm UI for an honestly documented open-source
release. Fix secret exposure, signing corruption, transaction correctness,
indexing integrity, packaging, and consequential coverage gaps. Include code
organization, public documentation, contract provenance, dependency/history review,
and an explicit support matrix. Preserve the successful vendor withdrawal and
recent node/registration/reference fixes.

The original request authorized planning artifacts only. On 2026-10-26 the user
authorized rebasing 001 onto master and tackling its implementation. Publishing a
repository, changing visibility, creating a release, using wallet secrets, and
signing/submitting transactions remain outside this implementation authorization.
An experimental source release
may document unsupported workflows, but must not advertise unsafe or unverified
operations as supported. Release owner decides final version and support scope.

## User Scenarios & Testing

### US1 — Create and sign with a wallet safely (P1)

An operator can save an encrypted wallet and add signatures without leaking the
recovery phrase or corrupting existing transaction data.

**Independent test**: Synthetic mnemonics and public transaction fixtures; no real keys.

1. Given an encrypted output request, when a wallet is created, neither stdout nor
   stderr contains its mnemonic unless secret display was explicitly requested.
2. Given a new plaintext wallet file, creation uses owner-only access; an existing
   file or symlink is not silently overwritten or followed.
3. Given definite, indefinite, or tagged supported witness encodings, adding a
   signature preserves original body bytes, prior witnesses, scripts, datums,
   redeemers, validity flag, and auxiliary data. Unsupported forms fail without output.
4. Given a published derivation/signing vector, the address, key hash, signature,
   and verification match the vector; repeated signing has documented semantics.

### US2 — Build correct portable treasury transactions (P1)

An operator can use every advertised workflow on supported instances, including
final withdrawals, without editing hardcoded deployment identifiers.

**Independent test**: Offline builder/CLI integration, real script evaluation, and
value/fee/min-UTxO/collateral boundary fixtures.

1. Given token-bearing wallet inputs, all non-minted/burned assets appear in outputs;
   if the builder cannot preserve them it rejects selection before writing a file.
2. Given the final scheduled payout and residual vendor value, withdrawal preserves
   that value and emits the continuing datum/output required by the contract.
3. Given another allowlist deployment, supplied/discovered references are verified
   against expected hashes; wrong, missing, duplicated or spent references fail.
4. Given multiple staking scripts, withdrawal entries and redeemer indices/budgets
   match ledger ordering, or the unsupported combination is explicitly rejected.
5. Given inadequate change, fee, min-UTxO, collateral or budgets, the builder either
   balances correctly or gives a specific error without emitting an invalid artifact.
6. Given the known successful withdrawal fixture, removal of the unused treasury
   reference still passes preflight, and redirecting its payout still fails.

### US3 — Trust portal events and project identity (P1)

An operator sees only justified events, correctly assigned to instances/projects
and milestones, regardless of similar names, payout counts, or payout ordering.

**Independent test**: Forged-metadata and multi-instance/milestone fixtures.

1. Given an unrelated transaction claiming another instance in label-1694 metadata,
   ingestion does not alter confirmed project/instance state.
2. Given two instances with identical project identifiers or payout counts, their
   projects, UTxOs, events, and deduplication records remain isolated.
3. Given a paused early payout and a matured later payout, withdrawal changes only
   the actual paid milestone; remaining paused statuses and dates remain correct.
4. Given ambiguous legacy project-to-output association, the portal exposes an
   unresolved state rather than guessing the nearest payout count.
5. Given registered but unconfirmed transaction data, registration remains distinct
   from chain confirmation and never submits it.

### US4 — Recover portal state across chain and process changes (P1)

An operator can restart, replay, roll back, or discover an instance without losing
history or retaining stale outputs.

**Independent test**: Mock chain sequences, persistent temporary databases, fault injection.

1. Given a rollback followed by replay, state equals a clean replay of the surviving
   chain, including derived milestones, deduplication and checkpoints.
2. Given rollback to origin or a stale checkpoint not on the chain, recovery uses
   a valid intersection and does not silently skip history.
3. Given a crash at any persistence boundary, restart does not skip an event whose
   writes were incomplete; no mixed slot/hash checkpoint is accepted.
4. Given reset of one instance, other instances retain their state and checkpoints.
5. Given an instance discovered after startup, configured supported backends begin
   indexing it without a manual restart; unsupported backend combinations fail clearly.
6. Given a spent/finally consumed vendor output, it stops appearing as unspent;
   chain and polling updates preserve native assets and real provenance slots.

### US5 — Operate the portal and UI predictably (P2)

A public-facing operator gets safe configuration and bounded resource use; UI users
can distinguish unavailable/unconfirmed data from confirmed withdrawals.

**Independent test**: HTTP/config/GraphQL integration and Elm decoder/state tests.

1. Given missing authorization configuration, write access is not silently enabled;
   unauthenticated operation requires explicit development configuration.
2. Given bind/network/backend conflicts or invalid limits, startup rejects them.
3. Given slow upstreams or expensive requests, configured deadlines and resource
   limits bound work and yield actionable errors, without false-success responses.
4. Given GraphQL errors, missing state, or a deployment-specific endpoint, UI behavior
   is correct and does not depend on a developer's localhost.

### US6 — Reproduce and maintain an open-source release (P1)

A contributor builds and tests a clean checkout and understands supported behavior,
contract provenance, reporting routes, and compatibility/migration obligations.

**Independent test**: Clean Nix builds, CI, documentation walkthrough, artifact inspection.

1. Given a clean checkout, CLI/portal/UI builds include required fixtures and run
   the same consequential tests as the development workspace.
2. Given a proposed release, formatting, strict lint, tests, package builds and UI
   checks pass; exclusions are explicit and cannot mask broken supported behavior.
3. Given a contract script blob, a contributor can identify its source revision,
   compiler/configuration and reproduce or verify its expected hash.
4. Given a report period spanning more than 100 GitHub results, MAF generation
   includes all available results or explicitly reports incompleteness/limits.
5. Given a release checkout, license/attribution, contribution/security guidance,
   support matrix, migration notes and accurate setup examples are present.

## Requirements

- **FR-001 [US1]** Suppress implicit secret output; secure wallet-file creation and replacement.
- **FR-002 [US1]** Preserve signing envelopes/witnesses losslessly or reject unsupported encodings.
- **FR-003 [US2]** Conserve all assets; handle empty payout lists independently of residual value.
- **FR-004 [US2]** Derive fee, min-UTxO and collateral requirements from supported protocol data;
  use checked arithmetic and validate amounts, addresses/networks and time conversions.
- **FR-005 [US2]** Make staking references configurable/discoverable and verified; correctly
  handle or explicitly reject multiple staking scripts and unsupported multisig requirements.
- **FR-006 [US2]** Establish validation and meaningful positive/negative cases for every
  advertised builder; document exact preflight limits and unsupported protocol versions.
- **FR-007 [US3]** Validate event provenance consistently across all ingestion paths.
- **FR-008 [US3]** Scope project/output/event/dedup identity by network and instance as needed.
- **FR-009 [US3]** Track output lineage and milestone identity; never infer authority/identity
  from payout count, lexical identifier order, or assumed withdrawal prefixes.
- **FR-010 [US4]** Roll back all derived state and replay markers; checkpoint only durable work.
- **FR-011 [US4]** Isolate reset, preserve/recover schema versions, and define a migration or
  explicit backed-up reindex path for existing installations without silent data deletion.
- **FR-012 [US4]** Supervise dynamic instance ingestion and reconcile spent outputs/assets;
  make node-only, Koios-only and mixed-backend support explicit.
- **FR-013 [US5]** Require explicit development mode for unauthenticated writes, support safe
  bind configuration, validate network settings, and enforce request/upstream limits.
- **FR-014 [US5]** Expose consistent confirmed/pending/unresolved/unavailable state to clients;
  validate GraphQL compatibility and configurable UI endpoints.
- **FR-015 [US6]** Include all test sources/fixtures in release builds and automate required gates.
- **FR-016 [US6]** Separate CLI wiring from workflows, consolidate duplicated transport/encoding,
  replace swallowed correctness errors, and split MAF responsibilities without broad rewrites.
- **FR-017 [US6]** Paginate MAF data retrieval, report truncation/rate limits, and test parsers/output.
- **FR-018 [US6]** Provide licensing, contract provenance, security/contribution/release docs,
  dependency and history review, and a supported-command/network/backend matrix.

## Key entities

Wallet artifact; unsigned/signed transaction envelope; full multi-asset value;
verified script reference; network/instance/project identity; vendor output lineage;
milestone-to-payout association; observed/validated event; chain point; derived
projection; schema version; release support matrix and validation evidence.

## Edge cases

Tagged/indefinite CBOR; duplicate signatures/redeemers; malformed signed bodies;
existing/symlink wallet paths; zero/negative/overflowing quantities; leftover ADA or
unscheduled tokens; large datum/token bundles; multiple reward scripts; paused and
unsorted payouts; identical projects across instances; arbitrary on-chain metadata;
rollback to origin; stale intersections; crash between stores; missing UTxOs versus
upstream failure; instances registered while running; >100 GitHub results; offline
builds; fresh databases versus existing installations.

## Success criteria

- **SC-001** No implicit mnemonic disclosure or witness loss in the regression matrix.
- **SC-002** Every supported builder passes asset, script, fee/min-UTxO/collateral and
  failure cases; the established public withdrawal remains valid under local evaluation.
- **SC-003** Portal state after rollback/restart equals clean replay for the test corpus;
  cross-instance and forged-event tests cannot change unrelated confirmed state.
- **SC-004** A clean checkout passes all declared release gates and includes fixtures.
- **SC-005** All review findings have implementation/test evidence or an explicit,
  user-accepted scope exclusion; mandatory secret/signing fixes cannot be waived by labeling.
- **SC-006** Release documents accurately describe limitations and reproduction steps.

## Assumptions and exclusions

Retain Rust/Elm and pinned dependency versions unless a researched fix requires a
change. Initial platform target remains the flake's x86_64-linux. Apache-2.0 is the
existing manifest intent, subject to attribution/dependency review. No contract
redeployment, production migration, mainnet automation, arbitrary performance SLA,
or release publication is part of this planning request. Storage strategy and
legacy milestone migration require focused design decisions recorded in research;
do not invent stable on-chain identifiers the contracts do not supply.
