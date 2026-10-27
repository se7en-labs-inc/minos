# Research and review evidence

## Implementation resume — 2026-10-26

The user authorized rebasing and implementing 001. `git rebase master` completed
cleanly in the existing worktree, moving it from `85eb785` to
`27ba8f889932dc983697c1884c6d871f8e81dcc2`. There were no branch-only commits;
the untracked Spec Kit artifacts and AGENTS.md survived. Master's local reports,
wallets and transaction artifacts were left in place.

The baseline now includes the MAF and portal/airgap/lifecycle additions on master.
The original findings below remain a historical review, not a fresh assertion
that every defect still exists at the new baseline:

- R12: `perSystem/packages.nix` now includes `minos/tests` and GPG/OpenSSL check
  inputs. Clean package execution of integration tests still needs verification.
- R22: new airgap, assembly, completion, mint, multisig, project template,
  treasury funding and wallet-address tests exist; broad coverage obligations
  remain as described by the feature tasks.
- R02: signing now explicitly rejects indefinite witness maps/sets rather than
  silently dropping them. The full signing compatibility matrix remains pending.

T001 checks started through the pinned project Nix environment:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `nix flake check --no-build` | Evaluation passed; this does not execute checks. Warnings: exported `inputs` and Crane placeholder name/version for workspace formatting. |
| `cargo test --workspace --locked` | Passed: 63 tests; the portal compiled after the approved lint cleanup |
| `cargo clippy --workspace --all-targets --locked -- --deny warnings` | Failed (exit 101): 23 existing portal diagnostics; Minos targets had no reported errors |
| `nix build .#minos .#minos-ui --no-link` | UI passed; CLI integration tests ran; portal test executable failed to load `libstdc++.so.6` |

UI artifact: `/nix/store/qz06d887wr9mf76zlfzfacq0dwamfnbb-minos-ui-0.1.0`.
Clippy diagnoses unused imports/code/fields in portal `db.rs`, `indexer.rs`,
`koios.rs`, `parser.rs`, `types.rs`; deprecated `lovelace_amount()` in `sync.rs`;
redundant closures and clone-on-Copy in `schema.rs`/`sync.rs`. User approved
targeted T003 fixes after the failure. Removed unused private helpers/response
fields/imports and the empty NixOS template module, marked the registration
inspection helper test-only, and updated deprecated/redundant expressions.
Existing transaction-hash field validation is retained when discarding unused
response data. Strict workspace Clippy then passed without warning suppression.

T004: added `.github/workflows/ci.yml` with read-only repository permissions,
commit-pinned checkout/Nix installer actions, locked Cargo checks and non-updating
Nix lockfile use. It runs Rust formatting, strict workspace lints, workspace tests,
CLI/portal/UI package builds and flake checks. Validated with project-pinned
`nix shell --inputs-from . nixpkgs#actionlint --command actionlint
.github/workflows/ci.yml` (passed). No remote workflow run or push was performed;
all configured Rust commands now pass locally. Hosted execution remains untested.

T005: wallet creation now requires `--out` or explicit `--show-mnemonic`. Saving
prints only a path unless display is explicitly requested. Plaintext/encrypted
wallet files use Unix mode 0600 and `create_new`, refusing existing files and
symlinks. GPG output is captured and checked before file creation, stdin is closed
before waiting, and failed encryption creates no wallet output. Five new tests
use only disposable/synthetic wallets and an isolated GPG keyring; they cover
stdout/stderr, explicit display, round trip, permissions, no-clobber, existing and
dangling symlinks, directory/missing-parent errors and failed encryption.
`cargo test --workspace --locked` now passes all 68 tests. Strict workspace
Clippy, Rust formatting and `git diff --check` pass after these changes.

T007/T008: signing splits a definite four-item envelope into raw slices, validates
the Conway shape, and rebuilds only the witness map. It preserves non-key witness
bytes and signed body/auxiliary/validity bytes, including a non-minimal body
encoding. Tagged definite vkey sets retain tag 258. Identical repeated signing is
byte-idempotent; conflicting signatures, duplicate map/key witnesses, invalid
key/signature lengths, unsupported witness keys/tags, malformed envelopes and
trailing bytes fail before output. Indefinite witness maps/sets remain explicitly
unsupported and fail closed. Replaced hand-sized CBOR header encoders with
minicbor's length encoders. Five signing tests cover synthetic complete witness
sets, public withdrawal regression, partial/repeated signing and unchanged or
absent CLI output after rejection. An initial test assertion borrow error was
corrected; dedicated signing tests and all 73 workspace tests then passed, along
with strict workspace Clippy.

Additional release-gate failures and approved remediation:

- Full `nix flake check` initially failed treefmt. The generated config used
  nixpkgs Rust 1.95/edition 2024 while Cargo uses pinned Fenix Rust/edition 2021.
  User approved matching the formatter command/edition to Cargo and applying
  existing Nix/TOML/Elm formatters. `nix fmt`, Cargo formatting and diff whitespace
  checks then passed. A full `nix flake check --no-update-lock-file` rerun passed
  (strict Clippy, Rust format, treefmt and UI package checks).
- User approved the package runtime fix. Added the C++ runtime library and RPATH
  to the final package without invalidating dependency-build caching, and wrapped
  Minos with GPG on PATH. Portal tests now start without dev-shell library paths.
- The rebuilt package then failed its portal registration test: creating a JSON
  payload column tries to auto-install DuckDB's JSON extension, unavailable in
  Nix's sandbox (`/homeless-shelter`). No test was disabled and no database was
  migrated. The user subsequently authorized the fix. Enabled the pinned
  `duckdb` crate's `json` feature, which forwards to `libduckdb-sys/json`; its
  `build_bundled_cc.rs` includes the JSON sources and defines
  `DUCKDB_EXTENSION_JSON_LINKED`. No dependency version/lockfile or schema change.
  Added a portal-store regression asserting JSON's `(BUILT-IN)` install path,
  disabling extension auto-installation, creating the actual event table and
  round-tripping structured JSON through event append/query. All 74 workspace
  tests pass, including the new
  built-in JSON regression and authenticated/idempotent portal registration;
  strict workspace Clippy also passes. Clean `nix build --no-update-lock-file
  .#minos .#minos-ui --no-link` now passes. The final package log confirms all
  74 tests ran successfully in the sandbox, including the built-in JSON regression
  and all CLI fixture integrations. Full flake checks also pass with JSON enabled.
  Installed CLI help and portal config-error startup were exercised with
  `LD_LIBRARY_PATH` unset; the package closure includes the C++ runtime and GPG.

Verified artifacts for the DuckDB fix:

| Artifact | Store output | NAR hash |
|---|---|---|
| CLI + portal | `/nix/store/8zrix7sj3nzsznki4d286b6r4bcyfhap-minos-0.1.0` | `sha256-gmy/WN+Xgv4LKimMzGmoQCdxWpl1T+oYubK96K1gDyE=` |
| UI | `/nix/store/lcyw7i0fqdlqzfc85s4pf8hbrx5wdmcz-minos-ui-0.1.0` | `sha256-GdGoARKdh4J0b8wJQIn2qlwSUkHrY5mpsJEtVFu7mQ4=` |

Earlier source-research limits (subsequent resolutions below):

- T011 source research: agentic-nix inventory lists the contract repository, but
  semantic search failed due to its unavailable embedding service and the sibling
  checkout is absent. Subsequent exact-text BM25 retrieval works; see below.
- Fetching/indexing pinned `duckdb` 1.10505.0 also failed due to the unavailable
  embedding service. The user's subsequent fix request authorized continued
  investigation: Linux-target Cargo metadata resolved the exact pinned local
  manifests/build source, confirming the JSON feature and static-link mechanism.
  An unfiltered offline metadata query initially requested uncached Android-only
  dependencies; filtering to the actual Linux target resolved from the cache.

## Privacy review — current tree and reachable history

User requested removal of personal filesystem paths and review of identifying
material, then explicitly accepted Git author/email attribution. All 17 reachable
commits were inspected for author/committer metadata and identifying text paths;
their accepted attribution remains intact. Package maintainer attribution remains.
No history was rewritten.

Current documentation uses generic socket paths, projects, signatories, deployment
placeholders and evidence filenames. Removed personal wallet/key mappings,
mainnet/preview operation handoffs and private report references from development
context; replaced that context with portable architecture/check instructions.
Test metadata uses synthetic project/vendor names and synthetic evidence CID.
No personal home-directory paths remain in the intended source tree.

Residual material and limits:

- Historical public-chain regression fixtures retain transaction bytes, addresses,
  keys, datums and deployed script linkage required by the real-contract tests.
  Public chain data is linkable; these fixtures are not anonymized. Fixture
  provenance and expected destinations in tests remain accurate.
- The MAF generator retains its existing legacy report-host normalization for a
  public project, preserving the behavior added on master. This organizational
  domain remains in source; synthetic email examples use reserved example domains.
- Older commits still contain the removed development context, deployment mappings
  and report details. Cleaning the working tree does not clean reachable history.
  Any removal from history needs a separately reviewed rewrite/export decision;
  accepted author/email attribution does not imply acceptance of all old file data.
- This targeted identity review is not a completed credential-history scan,
  dependency/advisory/license review, or a scan of ignored/local wallet/report files.
  T037 and final release acceptance remain open.

No live transaction signing/submission, publication, history rewrite or production secret reads
were performed. Implementation tasks remain unchecked until individually verified.

## Original review

Baseline: 85eb785, review conducted 2026-10-12. Paths below are baseline-relative;
line numbers may shift during implementation. Findings include recent assistant
changes and are not an attribution of every defect to the original author.

## Evidence ledger

| ID | Finding and evidence | Impact / proposed action | Requirement |
|---|---|---|---|
| R01 | `minos/src/main.rs:2039` prints mnemonic even after encrypted save; `wallet.rs:146` uses ordinary file writes | Secret exposure; explicit display, private creation, no accidental overwrite | FR-001 |
| R02 | `minos/src/tx/sign.rs:50` skips indefinite maps; line 63 treats indefinite arrays as zero length | Reproduced public fixture with valid indefinite witness map: redeemers present before, absent after successful witness insertion; lossless handling or rejection | FR-002 |
| R03 | `minos-portal/src/sync.rs:203` accepts instance-claimed metadata without contract/author linkage | Unrelated transactions can poison confirmed state; validate provenance | FR-007 |
| R04 | `db.rs:201` rolls back only events; sync-derived events have slot zero | Stale projections and dedup prevent correct replay; reversible projections and durable checkpoints | FR-010 |
| R05 | `db.rs:65` instance reset drops the entire events table and clears global dedup | Cross-instance data loss; scoped reset and migration/reindex policy | FR-011 |
| R06 | `sync.rs:183` dedup is globally keyed by hash while each instance scans blocks | Another instance's scanner can cause missed transactions; coherent scoped processing | FR-008, FR-010 |
| R07 | `db.rs:133,167` stores projects and outputs by project ID only | Same identifiers across instances overwrite; composite identity | FR-008 |
| R08 | `indexer.rs:394,436`, `schema.rs:494`, `parser.rs:118` rely on payout counts/prefixes/positions and lexical ordering | Wrong project, dates, pause and withdrawal status; lineage and explicit associations | FR-009 |
| R09 | `minos/src/main.rs:1175` removes output when payout list empties even with residual assets | Final payout blocked by conservation/preflight failure; use residual value | FR-003 |
| R10 | `coin_select.rs:26` may select tokens; `tx/withdraw.rs:176` returns ADA-only change | Invalid balance; asset-aware selection/change across builders | FR-003, FR-006 |
| R11 | `main.rs:798` fee estimator and fixed output/budget values; signed casts and saturating arithmetic elsewhere | Boundary failures; protocol-driven calculation and strict validation | FR-004, FR-006 |
| R12 | `perSystem/packages.nix:20` excludes `minos/tests`; unit tests include those fixtures | Confirmed evaluated source has references.rs but lacks fixture; packaging/test inclusion blocker | FR-015 |
| R13 | Strict Clippy command fails in library and test targets (10 diagnostics reported) | Existing configured check is red; fix causes without blanket suppression | FR-015, FR-016 |
| R14 | `main.rs:1022` maps only one deployment's allowlist hash/ref; reward helper replaces withdrawal map and uses index zero | Not portable; test/configure supported scripts and reward ordering | FR-005 |
| R15 | Portal `main.rs:81` starts sync only for config instances with publish_tx known at startup | Fresh/dynamic instances lack follow task; supervisor and bootstrap | FR-012 |
| R16 | `indexer.rs:246` upserts without deleting spent outputs; `sync.rs` writes ADA-only UTxO assets and slot zero | Stale/incomplete balances; consistent authoritative projections | FR-012 |
| R17 | Portal empty author list disables auth, binds all interfaces; no configured GraphQL cost limits; portal/MAF HTTP clients lack total deadlines | Unsafe public defaults/unbounded work; explicit dev mode and bounded APIs | FR-013 |
| R18 | Large main.rs (~2550 lines), MAF (~1100), Elm Main (~1019); repeated HTTP/CBOR/metadata code and swallowed writes | Difficult testing and hidden corruption; extract coherent services and propagate errors | FR-016 |
| R19 | `maf.rs:53,76` requests only first 100 search/release results | Silent incomplete reports; pagination and limit reporting | FR-017 |
| R20 | Elm has no tests, assumes localhost unless JS override; decoder hides some absent/malformed state | Test API/error states and explicit deployment configuration | FR-014 |
| R21 | No tracked LICENSE, CI, contribution/security policy; scripts are embedded hex; CONTEXT.md is stale | Public release hygiene, provenance and accurate supported-scope docs | FR-018 |
| R22 | 28 tests cover node(14), UPLC(3), refs(2), withdrawal(7), registration(2) | No dedicated wallet/signing, other-builder, replay/rollback, MAF or Elm tests found; behavior matrix required | FR-002, FR-006, FR-010, FR-014, FR-017 |

## Verification already performed

- Workspace build and all 28 tests passed in the Nix shell at baseline earlier in
  this session. Formatting check passed during review.
- Strict Clippy failed; this is recorded as a defect, not a reason to disable checks.
- Nix source omission was inspected at the exact evaluated source path. A complete
  clean Nix release build was not run during the review.
- A temporary Rust probe reproduced R02 using only public fixture bytes and dummy
  witness bytes. It was removed from the checkout; recreate it as a regression.
- The user reported a successful actual withdrawal after the reference fix. This
  supports that path, not blanket ledger coverage of all builders.
- Fixture scripts match treasury-contracts blueprint functions after configuration;
  see `minos/tests/fixtures/withdrawal/README.md` for source revision evidence.
- No production code changed during review, no wallet secrets were read, and no
  transaction was signed/submitted by the review. No full secret-history,
  dependency-advisory, licensing, or live rollback audit has been completed.

## Decisions and implementation research gates

1. **Retain existing stack and pins initially.** Do not upgrade Pallas/uplc merely
   for cleanup; the current evaluator bridges different Pallas versions via CBOR.
2. **Signing representation.** Preserve raw signed body bytes; typed envelope
   traversal is preferred where it preserves encodings. Test tagged/indefinite
   forms first, then document exactly which forms are supported or rejected.
3. **Storage boundary (resolve before US4 mutation).** Existing Sled + DuckDB cannot
   share an atomic commit. Evaluate one authoritative transactional event journal
   with rebuildable projections versus consolidation into a single store. Record
   choice, crash invariants, migration and backup strategy; no silent database swap.
4. **Legacy milestone identity (resolve before US3 projection changes).** Vendor
   datums do not encode a universal project/milestone ID. Inspect actual metadata
   ordering and contract transaction lineage. Define deterministic association and
   explicit unresolved results; never fabricate identity from count or sorting.
5. **Event authority (resolve before ingestion changes).** Define requirements per
   event type: actual validated transaction inputs/outputs, instance registry and
   author policy as applicable. Distinguish evidence submissions from fund movements
   and never treat an authorized registration as chain confirmation.
6. **Fee correctness.** Verify current supported ledger rules in pinned source,
   including reference-script pricing tiers, witness count, datum size, collateral,
   and body-size changes after fee insertion. Record exact protocol coverage.
7. **Support scope.** Mainnet/preview CLI and declared portal backends need a concrete
   matrix. Other networks/platforms remain unsupported unless tested. Mark unsupported
   operations explicitly rather than claiming they pass a restricted evaluator.
8. **Release ownership.** Final version, public repository destination, security
   contact, and any scope reductions are maintainer decisions before publication.
   They do not block implementing the documented technical fixes once authorized.

## Continued implementation evidence — 2026-10-26

T006: published CIP-0003 Icarus roots (including the `foo` passphrase), pinned
cardano-addresses enterprise/base golden addresses, and RFC 8032 section 7.1
TEST 1 now anchor derivation, credentials and the signature primitive. Fixture
README records provenance and separates primitive signing from extended-wallet
signing. Wallet signatures verify against the published address credential;
changed messages fail. Account indices >=2^31 and network IDs other than 0/1 fail.
An RFC seed transcription typo was corrected from the primary document. Dedicated
wallet and complete workspace test runs pass.

T011: exact-text contract retrieval succeeded. The pinned upstream source is
SundaeSwap-finance/treasury-contracts revision
`84e105a9ff797fdf9873e0a5b71df9f108d8c008`, specifically
`validators/vendor.ak` and `offchain/src/treasury/withdraw/index.ts`.
The validator requires a continuing vendor output whenever input value minus
matured payouts is nonzero, regardless of remaining payout count. The CLI now
uses checked complete values and keeps the original vendor authorization with an
empty payout list in this case. A builder regression evaluates the actual pinned
vendor and allowlist scripts for zero residual, residual ADA, and residual ADA
plus unscheduled tokens. It also supplies a fee input with an unrelated token and
verifies that token returns to wallet change. All three scenarios pass.

T013/T014: removed the deployed hash/location dispatch table. The CLI accepts
repeatable `--script-reference SCRIPT_HASH=TX_HASH#INDEX`. The existing resolver
still checks complete responses, registry NFT, and hash of language-tagged script
bytes. Tests relocate the public allowlist bytecode to another transaction/index,
and verify missing/duplicate/malformed configuration fails. Multiple reward
scripts are explicitly unsupported: completion/withdrawal builders reject them,
and the CBOR helper refuses to replace any existing withdrawal map. Existing
single-script real-script regressions remain green.

T028: portal defaults to loopback binding. Empty author configuration now fails
unless the operator explicitly enables development_auth_bypass on loopback.
Validate author/policy hashes, backend presence/URLs, positive ports/intervals,
network magic, known Koios hostname consistency, and instance network consistency.
Configuration and registration tests pass. This is an explicit configuration
migration: old empty-author development setups must opt in to the bypass.

### T009 ledger calculation research (before calculation changes)

Primary source pin: IntersectMBO/cardano-ledger tag
`cardano-ledger-conway-1.22.1.0`, resolving to commit
`226b002d5b5e83e24355f8a28ab214f3259eabda`. Source paths below are relative to that
commit, retrieved after discovering symbols through agentic-nix BM25:

- `eras/conway/impl/src/Cardano/Ledger/Conway/Tx.hs`: getConwayMinFeeTx adds
  alonzoMinFeeTx and tierRefScriptFee. Each 25,600-byte tier increases byte price;
  sum rational costs, then floor the total reference fee.
- `eras/conway/impl/src/Cardano/Ledger/Conway/PParams.hs`: Conway's fixed tier
  multiplier is **1.2**, stride 25,600, maximum reference scripts per transaction
  200 KiB. The upstream unit test's multiplier 1.5 is a parameterized test case,
  not the Conway constant.
- `eras/alonzo/impl/src/Cardano/Ledger/Alonzo/Tx.hs`: linear size fee uses encoded
  transaction size; execution fee uses all declared redeemer units. Execution
  prices are rational and the combined execution cost rounds upward.
- `eras/babbage/impl/src/Cardano/Ledger/Babbage/TxOut.hs`: min coin is
  `(160 + encoded output bytes) * coinsPerUTxOByte`. Inline datum, native assets,
  address shape and reference-script bytes all contribute. Changing coin or fee
  can change CBOR widths, requiring iteration.
- `eras/alonzo/impl/src/Cardano/Ledger/Alonzo/Rules/Utxo.hs`:
  `100 * collateral balance >= fee * collateral percentage`; minimum collateral
  rounds upward. Babbage permits native assets only when all are returned; Minos
  currently restricts collateral to separate ADA-only inputs.

Calculation implementation must retain exact node price numerator/denominator,
reject absent/invalid required protocol parameters, measure actual witness-set
headers rather than add 107 bytes per witness, and rebalance until encoded fee and
change stabilize. Supported calculation rules are Conway protocol majors 9–11;
the pinned Conway Era source explicitly includes protocol-11 transitions. Future
eras require verified rules before extending this range. Evaluator and fee-rule
compatibility are separate capabilities. Local evaluation remains distinct from
full ledger checks.

### Later verification and release-audit progress

All 106 workspace tests passed after shared HTTP/CBOR/permission bounds, MAF
pagination/refactor, ledger calculations and scoped database/reset changes.
Strict workspace Clippy and whitespace checks passed at that checkpoint. Final
package artifacts remain to be rebuilt after all feature changes.

T020: schema 2 stores an explicit network and rejects legacy, mismatched-network or
newer schemas. Composite JSON-tuple keys scope projects, vendor outputs and dedup;
DuckDB event identity and queries include the instance. Tests prove collision
isolation, legacy local-registration preservation on refusal, and reopen/version
checks. A restartable scoped reset now preserves unrelated instances and local
registrations; this is not yet complete rollback/replay implementation.

T029: shared clients now apply connection10s/total30s deadlines, status checks and
streamed16MiB body limits to both CLI and portal Koios, portal GraphQL and MAF.
GraphQL limits are tested through the real HTTP server, including capacity32
saturation with immediate429, body1MiB and query complexity1000. Shared CBOR
prevalidation bounds transaction input to64KiB, depth64 and16384 items before typed
allocation; permission metadata is depth32/nodes512, rejects alias cycles and
invalid children rather than dropping them. Pinned permission object shapes,
legacy arrays and null-as-empty-AnyOf are supported.

T034: MAF fetching/parsing/rendering are separate modules. PR search follows pages,
checks total_count/incomplete_results, rejects >1000 results, duplicates and a
changing total. Releases paginate to exhaustion with an explicit100-page cap and
validate publication timestamps. Rate limits and other errors stop generation;
reports state source repositories, inclusive period and completed pagination.
Malformed sections, dates, links and evidence rows fail rather than silently drop
content. Tests retrieve101PRs and101releases, reject incomplete/malformed/limited
responses and malformed reports, inspect generated DOCX XML/relationships including
the last paginated records, and verify output write errors propagate.
Primary API behavior references:
https://docs.github.com/en/rest/search/search#about-search and
https://docs.github.com/en/rest/releases/releases#list-releases.

T035: Nix-built Aiken v1.1.19+e525483 reproduced the pinned treasury source's three
base validators byte-for-byte, including hashes; aiken.lock stayed identical.
Compiler revision e525483dd81e10861911a4ec320f05b30e586324, output
/nix/store/rs9sa7ywqg7wh5yvc9v3sjhx0ys4b9nv-aiken,
NAR sha256-OL1Tggmdm8D7IPEL/B6F9YRO77xuF6O0bWiBY+9dVT4=.
Offline regressions verify embedded assets versus independently extracted blueprint,
full historical applied treasury/vendor hashes and equality of their function ASTs
to those base scripts. LICENSE, NOTICE, contribution/security/changelog drafts and
contract reproduction documentation are now present. Package installs include
license/notice and CLI contract provenance. T036 remains open for support/operations/
UI/release documentation.

T037 initial scans (2026-10-26): Gitleaks8.30.1 scanned all reachable Git history
(16commits reported, ~1.07MB) and an explicit tracked-current-source export
(~1.29MB) with full redaction and no matches. No ignored wallets/reports/databases
were included. This is a pattern scan, not proof of absence or history cleanup.
A targeted identity scan in that export found only accepted Nix maintainer
attribution and the previously documented public legacy report-host normalization.
Author/email attribution is retained per the user's instruction. Historical private
operational context remains a separate reviewed rewrite/export decision.

Cargo-audit0.22.1 against RustSec revision
a78282a1414d6c28cc228c4eae3570451affb0dc (2026-10-26) reports13 vulnerability
entries: crypto-bigint0.5.5; rustls0.23.45 (four); rustls-webpki0.103.15;
webpki-roots1.0.9; and three ZIP extraction advisories for each of zip6.0.0 and8.6.0.
It also reports unmaintained fxhash/instant/paste and Tokio1.53.1 unsoundness warning
RUSTSEC-2026-0386 (patched1.53.3). TLS patch-update permission was requested under
the user's failed-check agreement; no security dependency update has been applied.

Reachability investigation so far: crypto-bigint is reached only by uplc's
cfg(target_family="wasm") k256 dependency; it is absent from the resolved supported
x86_64-linux graph. ZIP6 extraction is in libduckdb-sys's non-bundled download path;
this project compiles bundled+json, excluding that module. ZIP8 is used to write
MAF documents; no Minos operation extracts a supplied archive, and tests read only
named XML entries from their own generated documents. These findings do not waive
advisories or establish a completed audit. Runtime TLS findings require patches;
license/advisory policy and final review remain open.

## Portal reducer, recovery and UI implementation checkpoint

The portal now routes node blocks, Koios historical replay and manual indexing
through a shared deterministic reducer (`chain.rs`). A publish claim needs the
seed-derived pinned policy, exact REGISTRY mint/output, matching metadata-derived
script hashes and actual author witness when declared. Pending local registrations
remain separate. Vendor successors retain explicit payout/milestone identities;
withdrawal partitions actual matured active payouts and never infers a prefix.
Ambiguous associations are observations, not fabricated project events.

Sled batches commit a whole block's canonical projection/events, retained raw
relevant transactions and checkpoint together. DuckDB remains the query cache;
queries recover it from Sled. A regression drops the cache table after opening,
forces the post-Sled write failure, reopens and verifies confirmation, deduplication,
origin rollback and byte-equivalent replay with local registrations retained.
Seventeen portal tests passed at this checkpoint. Further ingestion, supervision,
migration and integration tests remain under their unchecked tasks.

`nix build .#minos-ui --no-link` passed with 12 Elm worker tests in its checkPhase.
The tests cover endpoint retention, loading/refresh, timeout/error recovery, empty
and pending results, API-version rejection, GraphQL failure, all supported statuses,
unknown-status rejection and exact u64 asset quantities as strings. Nix build log:
`fll9iafhx7mvsp258cx60cs5xr7150pq-minos-ui-0.1.0.drv`; resulting tested UI:
`/nix/store/fmqnmy3gdpmnl5fa1ml6jxw5zavqy6f3-minos-ui-0.1.0`.
The installed `config.js` controls the endpoint; its default is same-origin
`/graphql`. These results do not substitute for the final clean package gates.

### Verified portal and workflow follow-through

The 25-test portal suite now passes with the shared reducer on all ingestion paths.
Its synthetic lifecycle executes the pinned oneshot, treasury and vendor scripts,
then verifies ordered project identity, adjudication, paused-prefix/non-prefix
withdrawal, full project/event rollback and equality with clean replay. A validly
witnessed wallet-only fund claim in the same block is rejected; only the actual
treasury Fund creates the project. Concurrent valid instances remain isolated.
The API test verifies failed output queries hide stale unspent outputs, report
Unavailable and recover without mutating canonical state or inventing withdrawals.

Offline maintenance rehearsal verifies new-destination migration, source retention,
local registration retention, interrupted migration resume and portable Sled restore.
Short-lived maintenance opens disable Sled's periodic flush timer and explicitly
flush/drop handles; migration does not start the regular server/cache as a validation
side effect. Backup documentation explicitly requires retaining adjacent legacy
DuckDB files separately. The canonical store is now schema 3; API version 2 migration
and coordinated CLI/UI deployment are documented in `docs/operations.md`.

Eight CLI workflows now live in focused modules under `minos/src/workflows/` rather
than in the argument parser. Workspace tests passed following extraction. Shared
HTTP/CBOR/metadata paths replace the old portal sync converter and metadata-only
indexing. The CLI handles full u64 quantities and rejects unknown statuses; direct
adjudicate/modify output overrides reject mismatched/incomplete data and malformed
quantities. Fund builders reject lost ADA/target tokens, duplicate inputs/assets/IDs
and overflow. Additional native-mint and publish phase-two regressions pass.

`cargo clippy --workspace --all-targets --locked -- --deny warnings` passed after
these changes. Final format, clean package tests and complete workspace gates must
be repeated after remaining builder work; these incremental results do not establish
release readiness or substitute for the separately authorized live preview lifecycle.

### Publish size failure and proposed bounded deployment

The strengthened 2-of-3 administrator regression now checks actual protocol limits,
minimum-output ADA and collateral as well as phase-two execution. The atomic
registry + treasury reference + vendor reference transaction is **17,714 signed
bytes**, exceeding the pinned fixture's **16,384-byte** protocol limit. The test is
left failing; increasing the protocol limit or dropping its assertion would conceal
a real inability to publish this supported configuration.

Proposed correction (requires maintainer confirmation under the failed-check
working agreement): build a two-transaction unsigned deployment bundle. Transaction
1 consumes the seed, mints/locks REGISTRY, carries authenticated publish metadata,
creates the treasury reference and retains enough wallet change for transaction 2.
Transaction 2 spends transaction 1's deterministic change outref, creates the vendor
reference and returns all unrelated native assets and remaining ADA to the wallet.
Both outputs use encoded-size minimum ADA and each transaction independently passes
signed-size/fee/value checks. Validate transaction 1 against live inputs; validate
transaction 2 against the exact projected transaction-1 output, clearly declaring
its confirmation dependency. Neither is signed or submitted automatically.

A reviewable bundle should contain `publish.json`, `vendor-reference.json` and a
manifest with transaction body hashes, dependency outref, expected script hashes
and ordering. Portal publication confirmation remains distinct from reference
readiness: the existing reducer can confirm REGISTRY first and discover the later
vendor reference at its script payment credential. Koios history also finds the
second transaction by that credential. Missing references must fail clearly before
building dependent project transactions. Add a 2-of-3 positive protocol regression
for both stages and negative incomplete-deployment/value/size cases; retain the
oversized atomic transaction as a negative regression. No gate is waived.

### Further builder and supervision verification

Shared dynamic minimum-output calculation now covers complete, withdraw,
adjudicate and modify, supplying any ADA shortfall from wallet change. Selection
uses protocol-derived maximum fees and iterates the actual native-asset change
minimum as inputs are added. No fixed fee/minimum-change/destination-ADA constants
remain in these workflows or the wallet transfer workflow. Collateral selection
uses exact outrefs, allowing a distinct output from the same transaction. All
builders perform shared protocol/value validation before writing an artifact.
Network checks cover outputs and every resolved input/reference/collateral output.

Completion regressions execute the pinned contracts with a selected 2-of-3 signer
subset (positive), one signer (negative), and a script-only vendor with an explicit
metadata actor and the required allowlist reward script (positive/negative).
The lifecycle exercises actual adjudicate/modify builders and rejects removal of
their declared authorization. Native mint tests reject output/input network
mismatches. A 20-asset wallet bundle requires selection of an additional ADA input
for minimum change; overflow/insufficiency fail explicitly.

An incremental full workspace run passed **140 tests**, with one failing target:
`multisig_publish` still reports **17,714 > 16,384 signed bytes**. This remains a
mandatory gate. Strict all-target workspace Clippy passed; subsequent small test
and collateral-selection changes require the final repeat. These are development
shell results, not a final clean package build.

Portal now gracefully cancels replay/sync, closes node clients and joins workers.
A real Unix-socket Pallas handshake/intersection mock drops its first connection,
observes reconnect with checkpoint plus origin, chooses origin, sends rollback and
await, and verifies stale projections clear while local tracking survives. The
idle worker then shuts down promptly. A Koios supervisor test registers an instance
after startup, observes repeated failed replay attempts, verifies no false checkpoint
and joins on shutdown. Missing/malformed/repeated reset instance selectors fail
before any reset. Node deadlines are bounded to 1–60 seconds and poll intervals to
1–86,400 seconds. API v2 `indexTransaction` queues durable tracking instead of
replaying full history inside its bounded HTTP request.

The latest complete portal run passed 30 tests. A prior full run intermittently
failed immediate reopen of a migrated Sled database with a lock error. This is not
considered fixed by a subsequent pass. Approval is pending for a process-boundary
migration rehearsal matching the offline CLI workflow, retaining all preservation
assertions. T027's implementation exists but final migration acceptance remains open.

Repeated Gitleaks 8.30.1 history scan inspected 16 commits (~1.07 MB), with no leaks
reported. Blob history still contains personal-path references in `CONTEXT.md`,
`README.md` and `specs/pallas-node-backend/handoff.md`; the relevant introducing or
changing commits are `2d7cee3`, `3d14848` and `85eb785`. The current source scan found
no such personal paths; the approved Nix maintainer handle remains. Author name and
email are explicitly retained. No history rewrite has occurred. These findings
must be addressed before claiming sanitized publishable history.

The offline cargo-about audit failed before license enumeration because Cargo
requested an uncached Android crate despite target filtering. Approval is pending
for a locked online audit. TLS advisory patch updates and the two-stage unsigned
publication design also remain pending. No advisory suppression, protocol-limit
relaxation, history rewrite, signing, submission or publication was performed.

Final validation follow-through added the protocol's maximum encoded value size:
Pallas node parameters retain `max_value_size` as the Koios-compatible `max_val_size`,
and shared protocol checks reject output values above the declared limit. The
fixture's added 5,000-byte bound is explicitly synthetic in its attribution.
The native-mint regression rejects an oversized encoded asset value. The latest
workspace run now passes **143 tests**, with the same one publish-size failure;
all31 portal tests and strict Clippy pass. The clean Nix Rust package gate is being
run with that failure retained, rather than disabling it. Its result remains open.

The clean Nix Rust gate completed unsuccessfully at the same retained publish-size
regression. Derivation `yigfywfmvdcw0yminpivbr56ghlv20ia-minos-0.1.0.drv` built
both release binaries and ran the preceding integration tests with fixtures from
the filtered source; no new source/runtime failure appeared before Cargo stopped
at `multisig_publish` (17,714 > 16,384 bytes). The later portal package tests did
not run after that failure. The dependency artifact
`sccgkp0h13mnnd0s7pmj1jyki1nv9c4i-minos-deps-0.1.0.drv` is now cached.
The subsequent documentation installation layout evaluates as
`9yjrdvsk831snd5mxl53lgsdflbljd98-minos-0.1.0.drv`, reusing the same dependency
derivation; its final install/artifact verification remains blocked on the failed
test. No successful final Rust output or complete package gate is claimed.

### Approved remediation implementation

The user approved the four-part remediation request. TLS lock patches applied:
rustls 0.23.46, rustls-webpki 0.103.16, webpki-roots 1.0.10 and Tokio 1.53.3.
A newly reported yanked yoke-derive 0.8.3 was updated to 0.8.4. Audit now reports
seven existing entries (Wasm-only crypto-bigint and ZIP6/ZIP8 extraction findings)
and unmaintained warnings; TLS, Tokio-unsoundness and yanked warnings are gone.
No suppression was introduced. Remaining reachability/acceptance decisions are
explicit in dependency review.

CLI publish now builds a two-transaction unsigned deployment bundle, preserving
all native assets through stage-one change and stage-two change. Both stages
measure signed size, balance protocol fees and output minima independently.
The first is preflighted against live inputs; the second is checked against the
exact projected stage-one change. The manifest binds both body hashes, script
hashes and ordered dependency/outrefs. A new directory is exclusive; manifest is
written last. Stdout is a structured bundle. No signing/submission is performed.
The 2-of-3 positive test passes while the oversize atomic form remains a negative
regression. Insufficient funds/collateral and missing dependency fail. Portal/API/UI
now distinguish reference readiness from confirmation and verify discovery and
rollback across the two stages. Automatic recovery of an expired second-stage
artifact is an explicitly documented CLI limitation.

The migration/restore rehearsal runs each operation in a subprocess and verifies
specific expected errors, unchanged source/backup records, exact restore contents,
local registrations, empty derived projections and interrupted-migration resume.
Its stronger exact-restore assertion exposed an empty internal metadata tree left
behind after marker removal. Restore now drops that tree only when the original
backup did not have it. The complete migration rehearsal passes without sleeps or
retries. Latest development workspace: **146 tests passed**, strict all-target
Clippy and treefmt fail-on-change passed.

cargo-about 0.9.0 completed the locked Linux license-file audit: 383 crates and
111 license texts. The generated root THIRD_PARTY_NOTICES.md omits local paths,
retains full texts/copyright and supplies source-version links. The notice collection
also contains all10 locked Elm upstream licenses, DuckDB 1.5.5's main license and
28 upstream third-party notice files, GNU license/exception texts and pinned native
runtime metadata. Downloaded bytes are recorded in licenses/SHA256SUMS. The package
installs notices with preserved relative paths. Final maintainer acceptance and
standalone runtime redistribution review are not inferred from this inventory.

The new UI package passes **14 Elm tests**, including confirmed-registry/pending-
reference and fully-ready states. The clean Rust package/flake gate is being rebuilt
following lockfile updates; its final artifact evidence will be recorded separately.
A disposable sanitized history candidate was prepared with no original ref/index
changes. Its final tree equals the feature snapshot; the author/email set is
unchanged. All17 reachable candidate commits pass Gitleaks and personal-home-path
search. The candidate bundle is for review, not authorization to replace actual
history or force-push. The actual repository history remains untouched.

Clean committed-candidate verification now passes: `nix build` and `nix flake check`
against the local Git bundle's exact revision, not an uncommitted worktree. All146
release-mode Rust tests and14 Elm tests pass; fixtures and notices are present.
Both installed binaries start without the development-shell library path, and
installed notice checksums match their manifest. Direct runtime references are
GCC's library output, Bash, GnuPG, glibc and the package itself; no previous Minos
package or old development artifact is pulled into the direct runtime references.
The source-only `release-results.json` records final output identifiers without
creating a circular reference in package inputs. Live T039 and the T040 advisory/
history/support decisions remain open. A preview-only fee address and private
unfunded role credentials are prepared; no socket or signing permission was supplied.
