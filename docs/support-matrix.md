# Supported transaction and portal scope

This is the implementation scope for release candidate 0.1.0. The release checklist
in `specs/001-open-source-release/checklists/release.md` records outstanding gates;
this document does not certify a release.

## Transaction construction and validation

Builders produce unsigned Conway transactions for Shelley addresses. The CLI
offers preview and mainnet; the portal also accepts preprod. Shared preflight
requires the caller's correct network/time configuration, a declared matching network, complete
resolved spending/reference/collateral inputs, conserved ADA and native assets,
validity bounds, protocol output minima, maximum encoded value size, measured signed size, minimum fee and
execution limits. Fees include distinct payment/required signers, execution prices
and tiered reference-script costs. Protocol versions 9–11 are accepted; other
versions fail explicitly. Parameters come from the selected backend at build time.

| Command | Contract behavior and authorization | Current validation |
|---|---|---|
| `publish` | Registry NFT/treasury reference, then vendor reference from the first transaction's exact change | Both unsigned transactions independently check protocol size, fees, minima and value; 2-of-3 positive regression; atomic oversize retained as a negative case |
| `fund` | Wallet deposit or treasury spend; milestone payouts, registry and required treasury reference | Value/asset conservation, protocol checks, pinned V3 script evaluation |
| `complete` | Preserve vendor datum and value; authenticated evidence metadata | Vendor key/threshold or one reward-script authorization; metadata actor required |
| `withdraw` | Matured active payouts to the specified destination; retain paused/future payouts and all residual value | Vendor authorization and pinned V3 evaluation; continuation follows residual value, including unscheduled assets |
| `adjudicate` | Admin/governance changes payout status; preserve value | Declared admin/co-signers and pinned V3 evaluation |
| `modify` | Correct vendor datum with governance and existing vendor consent | Bounded replacement datum, declared admin/vendor signers, optional reward script and pinned V3 evaluation |
| `mint` | Native single-signature policy, fungible token and optional CIP-68 reference NFT | Exact minted value, native policy signer, dynamic output minima and protocol checks |

`--vendor-signer HASH` (alias `--signer`) can be repeated to select a threshold
subset. Without it, signature leaves are declared. Script-only vendor authorization
also requires an explicit actor for metadata. Declaring a signer does not sign the
transaction: every required payment/authorization witness must be supplied before
submission. Referenced scripts are checked by hash at the resolved output; reference
outputs may live in separate transactions. Node mode uses filtered outref queries,
not a whole-ledger UTxO scan. Explicit legacy publish references remain available.

Only one distinct reward script is supported in a transaction. Multiple reward
scripts fail before construction; they are not silently overwritten. Current
preflight accepts Plutus V3 and zero-valued script withdrawals. Certificates,
governance actions, nonzero staking withdrawals, V1/V2 scripts, arbitrary native
policies and Byron outputs are outside this builder/preflight scope.

Preflight is not a complete ledger implementation. It evaluates unsigned
transactions using declared signers, not cryptographic witnesses. The node remains
authoritative for current UTxO availability, current parameters, transaction validity
at submission, witness verification and acceptance. An offline build cannot prove
that its inputs will remain unspent. Collateral currently uses one separate ADA-only
input without collateral return; phase-two failure can consume that input.

## Signing compatibility

The signer preserves the original transaction body and auxiliary data byte for
byte, retains non-vkey witnesses, and merges vkey witnesses without discarding prior
signatures. Tests cover definite/indefinite and tagged witness sets, idempotent
signing and malformed/trailing CBOR. Unsupported or conflicting witness structures
fail before a file is written. Transaction parsing is bounded to 64 KiB, depth 64
and 16,384 CBOR items. Wallet files use private, exclusive creation; printing a
mnemonic always requires an explicit flag.

## Portal and UI

Schema version 3 uses Sled as the authoritative block journal/projection store.
DuckDB is the embedded query cache; it needs no external database service and its
JSON support is bundled. The normal CLI defaults are Sundae's public mainnet and
preview portals; their legacy read schema is detected through introspection.
Legacy confirmation is unknown, and builders still check live chain inputs and
references. The bundled portal/UI are optional debugging tools selected through
an explicit endpoint. Their GraphQL API version 2 distinguishes pending,
confirmed, unresolved and unavailable data. Older API/schema versions require the
explicit migration/reindex procedure in [operations](operations.md).

Node-only sync starts at origin, resumes from its durable intersection, and falls
back to origin when its saved point is stale. `referencesReady` is separate from
registry confirmation: the UI displays references pending until both references
are confirmed, and rollback clears readiness. The manifest requires first-stage
confirmation before second-stage submission. Automatic recovery of an expired
second stage is outside the CLI scope; retain its public artifacts for replacement.
Koios-only mode retrieves complete
credential history with bounded pagination and rejects duplicate/inconsistent
responses or a changing tip; a long history on a frequently advancing tip may
require node mode. Mixed mode uses node history and snapshots; Koios also supports
explicit transaction-registration lookup. New tracked instances are supervised
automatically. Shutdown cancels sync/replay and joins workers.

Only on-chain transactions with validated instance identity, script execution,
metadata authorship and output lineage create confirmed projects/events. Local
registration is pending. Snapshot failures hide stale outputs without inventing
withdrawals. Legacy or ambiguous payout identity is unresolved. A missing output
alone is not evidence of a payout. Only initial ordered metadata or uniquely
validated lineage establishes a payout ID.

The packaged UI loads `config.js`; configure the endpoint there or proxy
same-origin `/graphql`. See [operations](operations.md) for bind/auth defaults,
timeouts, backup/restore/migration and deployment instructions.
