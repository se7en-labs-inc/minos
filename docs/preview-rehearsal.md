# Disposable preview acceptance rehearsal

This is the T039 acceptance sequence. Clean package gates pass. The maintainer
has authorized work on T039/T040, including fresh disposable-preview signing and
submission. The documented dependency scope is accepted; the maintainer deferred
the rehearsal for master integration. Execution still needs a preview node socket
and faucet funding. Current observations and outstanding inputs are in
`specs/001-open-source-release/acceptance-review.md`.
Publication now uses the two-stage deployment bundle; do not submit the
oversized atomic regression fixture. Migration/restore uses separate processes.

## Inputs and isolation

Use a fresh directory outside the repository, a preview node (magic 2), a fresh
schema-3 portal database and the matching packaged API-v2 UI/CLI. Use disposable
preview-only wallets for three administrators, a separate vendor and a fee payer.
Each role should have its own mnemonic for independent-key coverage. Do not reuse
the operational wallets, transaction files, reports or databases in another checkout.
Obtain test ADA only from the preview faucet; record public addresses and funding
transaction hashes, never mnemonic/encrypted-wallet contents or GPG secret keys.

The operator supplies preview node socket, GPG recipients, disposable wallet paths,
public payment addresses/key hashes, faucet transaction hashes and an explicit
signing/submission authorization. An existing funded wallet is not a substitute.
Record the final package output identifiers, Git commit and network in the evidence.

Inspect `minos --help` and each command's help from the tested package. Prepare
unsigned artifacts first, inspect every input/output/asset/datum, and verify body
hashes before the separately authorized signing stage. Use new output filenames
for each signature. Keep a public evidence ledger containing step, unsigned body
hash, submitted hash, confirmation slot/block, portal event ID and UI result.

## Lifecycle acceptance sequence

| Step | Action | Required result |
|---|---|---|
| 1 | Start portal with authorized authors and no preconfirmed deployment | Empty/pending state; no fabricated projects or events |
| 2 | Publish a fresh 2-of-3 instance with the corrected bounded deployment | Every required deployment transaction satisfies signed size, fee and min-ADA; registry confirms before reference readiness is claimed |
| 3 | Query registry and script outrefs through the portal; build using node mode | Hash-verified references, including scripts in separate transactions, with no full-ledger UTxO scan |
| 4 | Mint synthetic tokens and deposit a project budget | Token quantities and all unrelated wallet assets conserved; exact expected script destination |
| 5 | Fund two projects with IDs that collide across a second independent instance | Composite identity isolates both instances and both project histories |
| 6 | Submit completion evidence with two selected vendor signers or configured script authorization | Signature/metadata author validated; payouts and full vendor value retained; insufficient signer/script attempt rejected locally |
| 7 | Pause the first payout, mature later non-prefix payouts, then withdraw | Only matured active payouts paid to the reviewed destination; paused prefix and native assets remain; correct milestone IDs shown |
| 8 | Modify/resume the remaining payout with the required admin/vendor consent | Payout lineage retained where uniquely established; missing consent rejected; ambiguity remains unresolved |
| 9 | Withdraw the final scheduled payout with residual ADA/unscheduled tokens | Residual value stays at the vendor output with a continuation datum; no asset burned/lost |
| 10 | Register a new instance while the portal is running, then interrupt/restart its backend | Supervisor discovers it, retains committed state on failure, reconnects and catches up without duplicate events |
| 11 | Stop the portal, back up, migrate/reindex into a new database and restart | Local registrations retained, original/backup unchanged, rebuilt projections match clean replay |
| 12 | Replay the recorded public blocks with rollback to an earlier point and origin | All projections/checkpoints/events agree with clean replay; other instances and local registrations survive |

A public preview network cannot reliably be made to roll back on demand. Step 12
must use the controlled protocol/replay fixtures for deterministic rollback evidence;
record any naturally observed preview rollback separately. Do not describe fixture
results as live node observations.

## API, UI and failure observations

Query `apiVersion`, `confirmed`, `observations`, project availability, unspent output
and milestone statuses after each step. Verify the packaged UI against the same
endpoint and retain public screenshots/query responses with no wallet material.
Pending registration and submission acknowledgement must not display confirmation.
Interrupt output queries: stale unspent output must disappear and show Unavailable,
then recover without fabricated withdrawal events. Unknown legacy payout identity
must show Unresolved. Verify unrelated witnessed metadata cannot create a project.

A submission timeout remains an unknown outcome: look up the public body hash before
any retry. Never automatically resubmit. Preserve exact node rejection details and
stop for review if a required step fails. Attach the resulting evidence to the
release checklist and obtain the final maintainer support/version acceptance before
requesting publication.
