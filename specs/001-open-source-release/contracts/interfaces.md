# Behavioral interface contracts

## Wallet and transaction CLI

- Encrypted wallet output never implicitly prints its mnemonic. Explicit secret
  display has a documented opt-in interface; success messages contain paths only.
- New wallet files are private, no-clobber, and symlink-safe. Unsupported platform
  guarantees fail explicitly or remain outside the support matrix.
- Signing reads transaction envelopes losslessly, preserves body hash and existing
  witnesses, and fails before replacing output for unsupported/malformed encodings.
- Builders return either a balanced validated unsigned artifact or a specific
  failure. Validation/signing/submission remain distinct operations.
- Reference configuration specifies actual outrefs and expected credentials; no
  deployment-specific hash table is required for advertised supported instances.
- Protocol, address-network, arithmetic, min-UTxO, budget and collateral errors are
  distinguishable from transport failures. Submission uncertainty retains the
  existing transaction hash/no-automatic-retry behavior.

## Portal/API

- `registerTransaction` remains registration-only; `newTransaction` remains its
  documented local alias. Never fall back to a remote mutation that may submit.
- Authenticated registration proves envelope authorship, not chain validity.
- Every chain-ingestion entrypoint applies equivalent event-provenance rules.
- Existing query names/field types remain compatible where sound. New status or
  composite lookup fields must be explicit and coordinated with CLI/Elm clients;
  ambiguous legacy ID-only lookups fail rather than select an arbitrary project.
- Missing/unresolved/unavailable state is not reported as withdrawn. Pending and
  confirmed data are distinguishable. Public requests have documented bounds.
- Configuration explicitly controls bind address, development auth bypass, network,
  supported backend combination, deadlines and request limits.

## Storage and operations

- Replay results are independent of task scheduling, batch boundaries and restarts.
- Checkpoints identify durable validated work; rollback/reset operate on all related
  projections and markers without touching unrelated instances/local records.
- Poll snapshots reconcile absence only after a successful complete query. Transport
  failure cannot erase state or masquerade as an empty wallet/project.
- Schema changes expose version and migration/reindex requirements before mutation.

## Reports and releases

- MAF output states source scope, pagination completion and retrieval failures.
- Release commands from a clean checkout include tests/fixtures and produce the
  documented artifacts. Runtime dependencies such as GPG are declared or wrapped.
- Public source/artifacts exclude local wallets, transaction outputs, databases and
  private reports. History review is separate from working-tree ignore rules.

### Implemented staking-reference CLI contract

`minos --script-reference SCRIPT_HASH=TX_HASH#INDEX complete ...` and `withdraw ...`
accept repeatable explicit locations. Hex policy/script hashes are 28 bytes and
transaction hashes 32 bytes. Duplicate configured hashes fail. Required hashes
come from the vendor multisig datum; no configured location can replace the
expected hash. The fetched language-tagged bytecode must match, and every requested
output must be returned exactly once. Completion and withdrawal currently support
at most one distinct reward script and reject additional scripts before output.
Existing single-script deployments must now supply this option rather than rely
on the former built-in location. Signature-only vendor authorization needs none.

### Implemented portal request bounds

Default bind is `127.0.0.1`. Authorized registration keys are required unless
`development_auth_bypass = true` is explicitly configured with a loopback bind.
Existing development configurations with empty authors must opt in. Production
operators configure real 28-byte key hashes. Node, Koios, or mixed backends are
accepted; at least one is required, intervals must be positive, and instance
networks must agree with network magic. Known Koios network hostnames are checked.

GraphQL requests have a 1 MiB body limit, 32 concurrent requests, 30-second total
processing deadline, maximum query depth 16 and complexity 1,000. Outbound portal
and Koios clients use 10-second connection and 30-second total deadlines; shared
JSON response decoding stops at 16 MiB including chunked responses. These bounds
are application defaults, independent of deployment proxy limits.

### Confirmed-event provenance rules

Source pin: treasury-contracts `84e105a9ff797fdf9873e0a5b71df9f108d8c008`,
`validators/{oneshot,treasury,vendor}.ak`, `lib/types.ak` and
`offchain/src/metadata/{shared.ts,types/*.ts}`. The historical withdrawal fixture
independently anchors the vendor Withdraw and allowlist path; rebuilt base scripts
are verified in `docs/contracts.md`.

Every confirmed observation requires a successful transaction at a real chain
point on this database's network. The metadata must be bound by the body's
auxiliary-data hash; the claimed instance must match the validated registry policy.
A label-1694 object and required signer names alone are not proof of execution.
When txAuthor is present, it must be a valid witnessed/ledger-required signer for
this exact body. Global registration allowlists control local registration and
manual discovery; they do not replace a contract's permission tree.

| Event | Required linkage in the successful chain transaction | Projection authority |
|---|---|---|
| Publish | Consume the declared seed; mint exactly one REGISTRY under the policy derived by applying the pinned oneshot base to that seed; locate the registry NFT and inline registry datum; match actual treasury/vendor script hashes to the strictly parsed metadata configurations | Create a confirmed instance only after these checks; an authorized signed local envelope remains pending |
| Fund | Spend a resolved treasury output of this instance with treasury redeemer constructor 2; validate registry reference and actual vendor output/datum/value | Create project/output lineage; only an explicit ordered milestone list may establish index mapping |
| Complete | Spend a linked vendor output with vendor Withdraw constructor 0; validate its actual payout partition and continuing datum/value, txAuthor and milestone association | Attach evidence to the established project/milestone; derive any simultaneous matured withdrawal independently from the actual transition, never from the complete label |
| Withdraw | Spend a linked vendor output with vendor Withdraw constructor 0; partition all matured Active payouts using the validity interval; verify continuing datum/value or zero residual | Record precisely the removed, mapped payouts, including non-prefix removals and final consumption |
| Pause / Resume / Adjudicate | Spend linked vendor output with vendor Adjudicate constructor 1; compare statuses and unchanged payout value/maturation; check metadata claims against actual changed entries | Update only matched identities; mixed or missing labels cannot invent milestone association |
| Modify | Spend linked vendor output with vendor Modify constructor 2; validate actual successor outputs and treasury return | Follow unambiguous lineage; expose unmapped/replaced payout identities as unresolved |
| Sweep | Spend this instance's treasury constructor 1 or linked vendor constructor 3/4; verify actual destination and lineage | Record funds movement without inventing withdrawal/completion evidence |
| Reorganize | Spend this instance's treasury constructor 0; bind any declared output indices to actual outputs at its treasury credential | Update treasury output labels/lineage, never vendor milestone state |
| Initialize | An ordinary wallet transfer creates actual outputs at the already validated treasury credential; bind declared indices and authenticated author if supplied | Label an observed deposit; this does not prove treasury-script execution, create a project, or confirm a new instance |
| Disburse | Spend this instance's treasury constructor 3; verify actual output destinations/amounts | Record the supported treasury movement; no vendor milestone inference |
| Cancel / unknown extensions | No corresponding pinned metadata contract is defined | Reject as confirmed domain events until an explicit verified contract is added |

Resolve spending and reference inputs from accepted journal lineage or complete
provider responses; fail closed on missing, duplicate, conflicting or unexpected
results. Verify registry NFT, datum and script hashes rather than merely matching
an output address. A transaction that sends unsolicited funds to a vendor address
cannot assign those funds to a project from payout count or copied metadata.

All ingestion routes use the same validator: node block ingestion, Koios/manual
indexing and snapshot reconciliation. Rejected claims may be retained as bounded
observations with reason and chain point; they cannot mutate confirmed projections.
Ambiguous but otherwise valid lineage is explicitly unresolved. Local registration
and current UTxO availability are separate from confirmed history. Network/query
failure is unavailable, not an empty snapshot or a confirmed withdrawal.

## Bounded publication bundle (approved remediation)

`publish -o NEW_DIRECTORY` writes `publish.json`, `vendor-reference.json` and,
last, `manifest.json` into a new directory. No files are created before both
transactions are built/validated and the first is checked against live inputs.
Without `-o`, stdout is a JSON object containing both envelopes and the manifest.
This replaces the CLI's single atomic publish envelope; the library's atomic
builder remains for compatibility and the oversized negative regression.

Manifest format version 1 contains `networkId`, `instance`,
`treasuryScriptHash`, `vendorScriptHash`, `publishBodyHash`,
`vendorReferenceBodyHash`, `vendorReferenceDependsOn`, `registryReference`,
`treasuryReference` and `vendorReference`. References have `transaction` and
`index`. Registry and treasury reference are stage-one outputs 0 and 1. Stage two
spends stage-one change at index 2 and creates the vendor reference at index 0.
Body hashes remain unchanged during witness collection. Sign/submit stage two
only after stage one confirms. The prepared transactions share their validity
window; the CLI does not automatically recover an expired second stage.

GraphQL API 2 adds the derived `referencesReady` Boolean. It is true only when
publication is confirmed and registry, treasury and vendor reference outrefs are
known/current. It is distinct from `confirmed`; the UI shows references pending
between the two confirmed stages. Reference consumption/rollback clears readiness.
No additional storage schema is needed for this derived field.
