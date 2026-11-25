# Monthly MAF evidence submission

This runbook records the Se7en Labs mainnet workflow for CTW-26-0024. It starts
with the approved, signed MAF PDF and ends with chain confirmation and evidence
visible on the correct milestone. It does not generate the MAF document or claim
a milestone payment. M2.1 and M2.2 completed this flow. M2.2 reused the exact
M2.1 reference inputs, confirmed on chain and appeared on Sundae's m-8 evidence
entry on 2026-11-24 UTC.

## Tools and fixed deployment details

Use the installed, current `minos` on both machines. From this checkout, enter
`nix develop`, run `cargo build -p minos --locked`, and substitute
`./target/debug/minos` for `minos`. GPG and the encrypted mainnet mnemonic belong
on the airgap. No separate helper script, `cardano-cli`, Python, or local portal
is needed. The online commands use the network's Sundae portal and chain backend.

| Setting | Value |
|---|---|
| Network | `mainnet` (the CLI otherwise defaults to preview) |
| Instance | `51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e` |
| Project | `CTW-26-0024` |
| Vendor signer, account 0 | `5d55fd460b9f33d3860b46d2bfc166fdd40017015eae3a2a7e37241b` |
| Fee/change address | `addr1v9w4tl2xpw0n85uxpdrd907pvm7agqqhq902uw320cmjgxc0e87ue` |

These are public deployment identifiers. Do not put wallet contents or decrypted
keys in the repository. Recheck deployment state before every submission.

## Exact reference inputs

The successful M2.1 transaction
`e6f9f0221df2917cd4a034a0321b146f2e8007779fbc0eaab70e6b3b7dd57119`
and the prepared M2.2 transaction use these same three reference inputs:

| Purpose | Script hash | Output reference |
|---|---|---|
| Registry NFT and datum | No reference script | `fe2a314561f787aa55f2cf375438a7c8d4f5d090c82292c0e0d6699aec11e722#0` |
| Vendor validator, Plutus V3 | `5bab145a5e92c865725365bc8fa345abea7f292a12c85c951baf5bfa` | `c307d0fcd0b63f6e31d44d7bb1726661b949f9f50eac60be7f81fb6748d35472#0` |
| Allowlist reward validator, Plutus V3 | `326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb` | `aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8#0` |

`complete` discovers the registry and advertised vendor reference. Supply the
allowlist reference explicitly with `--script-reference`, as shown below. Minos
checks the live reference bytes against the expected hashes. The allowlist is
required alongside the vendor signature: `--vendor-signer` alone does not remove
that requirement. There are two evaluated redeemers: vendor spending and a zero
reward withdrawal for allowlist authorization. Do not add the unused treasury
validator reference, assume `publish-tx#2`, or redeploy scripts.

Inspect an earlier successful transaction without loading a wallet:

```sh
minos --network mainnet wallet inspect --tx mainnet-m21-airgap-ready.json
```

Its `body.reference_inputs` records the three references above. If they become
spent or change, stop and resolve the deployment before preparing another body.

## Prepare online

Run from the master worktree. Keep transfer files here so they are accessible on
the host. Exclude PDFs and operational JSON artifacts locally through Git's
`info/exclude`; do not commit them. Use fresh output names for later milestones or
retries and preserve earlier packages and signed responses.

Read current status first. Confirm the project, milestone label and existing
completions so an already submitted MAF is not duplicated.

```sh
minos --network mainnet status \
  --instance 51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e
```

M2.1 September 2026 is `m-7`; M2.2 October is `m-8`; M2.3 November is `m-9`.
Confirm later mappings from live labels. The reporting month differs from the
submission date and payout date.

Example for M2.2 (change PDF, milestone, description and output names together
for another month):

```sh
minos --network mainnet \
  --script-reference '326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb=aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8#0' \
  complete \
  --instance 51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e \
  --project CTW-26-0024 --milestone m-8 \
  --description 'M2.2 October 2026 maintenance completed; signed Milestone Acceptance Form attached.' \
  --evidence-file CTW-26-0024__Se7en_Labs_MAF_M22_October_2026.pdf \
  --vendor-signer 5d55fd460b9f33d3860b46d2bfc166fdd40017015eae3a2a7e37241b \
  --fee-addr addr1v9w4tl2xpw0n85uxpdrd907pvm7agqqhq902uw320cmjgxc0e87ue \
  --out complete-m22-mainnet-unsigned.json

minos --network mainnet wallet inspect --tx complete-m22-mainnet-unsigned.json
```

Preparation calculates the PDF's UnixFS CID and evaluates the scripts without
uploading or signing. Confirm the exact reference inputs, metadata target and
filename, sole required wallet signer, full vendor assets/datum preservation,
fee, collateral and validity window. Fees and spending inputs must come from live
state, not the previous month's transaction. Local preflight is not full ledger
acceptance. For this October PDF, the expected URL is
`ipfs://bafybeidwkeorhixae6pakyhha3ukv3sxgivbd4blbtekfd3sgelxpoezv4`.
Use `minos evidence cid --unixfs --file FILE.pdf` to calculate another file's URL.

When ready for immediate offline signing, prepare the combined request:

```sh
minos --network mainnet evidence prepare-package \
  --tx complete-m22-mainnet-unsigned.json \
  --file CTW-26-0024__Se7en_Labs_MAF_M22_October_2026.pdf \
  --out mainnet-m22-airgap-request.json

minos airgap review --package mainnet-m22-airgap-request.json \
  > mainnet-m22-airgap-review.json
```

The request embeds the transaction, PDF and exact upload authorization. Check the
review's `network` (1), `metadata`, `signer`, `remainingSigners` (empty),
`bodyHash`, `pdfSha256`, `evidenceImportProfile`, fee and UTC expiry. The profile
is `unixfs-cidv1-256k-raw-leaves`; a raw-file CID is different. Older completion
console output may misleadingly label this URL as raw; the package review reports
the actual import profile.

## Sign offline and return promptly

Transfer only the request to the airgap, which must have the current Minos with
UnixFS package support. Substitute the actual encrypted wallet path:

```sh
minos airgap review --package mainnet-m22-airgap-request.json
minos airgap sign --package mainnet-m22-airgap-request.json \
  --wallet /path/to/mainnet-wallet.asc --account 0 \
  --out mainnet-m22-airgap-signed.json
minos airgap verify --package mainnet-m22-airgap-request.json \
  --signed mainnet-m22-airgap-signed.json
```

One signing operation returns both the transaction witness and CIP-30 COSE upload
authorization. `--expected-request-hash` is optional. Offline checks do not depend
on the airgap clock; online validity checks still apply. Return only the signed
JSON, never the wallet.

The portal rejected an authorization older than two minutes during M2.1. Treat
that as an observed freshness constraint, separate from the transaction's longer
expiry; arrange the transfer before preparing the request. A refreshed timestamp
or nonce changes the request and needs a fresh offline signing round. Preserve
old files, and never pair an old signed response with a refreshed request.

## Verify, upload and submit online

After the signed response returns:

```sh
minos airgap verify --package mainnet-m22-airgap-request.json \
  --signed mainnet-m22-airgap-signed.json
minos --network mainnet preflight --tx complete-m22-mainnet-unsigned.json
minos --network mainnet evidence upload-package \
  --package mainnet-m22-airgap-request.json \
  --signed mainnet-m22-airgap-signed.json \
  --receipt mainnet-m22-airgap-upload-receipt.json \
  --out mainnet-m22-airgap-ready.json
minos --network mainnet preflight --tx mainnet-m22-airgap-ready.json
minos --network mainnet wallet inspect --tx mainnet-m22-airgap-ready.json
```

Require both signatures to verify, the upload receipt's `cidMatches` to be true,
the original body hash to remain unchanged, and fresh signed preflight to pass.
A CID mismatch or expired/spent transaction needs investigation and a new signing
round as applicable; never replace the evidence URL inside a signed body.

M2.1 uploaded successfully, but portal `submit --via-portal` registration rejected
the author despite its valid required signature. The user then authorized direct
Koios broadcast, which confirmed and was automatically indexed by Sundae. For an
authorized submission using that same route:

```sh
minos --network mainnet wallet submit --tx mainnet-m22-airgap-ready.json
```

Without a configured node socket, this broadcasts through Koios. `submit
--via-portal` only registers for signature collection; it does not prove chain
broadcast. Do not repeatedly attempt the known rejected registration as a
prerequisite for direct submission. Submission authorization is separate from
running documentation or tests.

Record the returned transaction ID. Confirm inclusion through chain observation
(for example, the new fee-change output via `minos utxos --address ADDRESS`) and
then re-run `status` to confirm the correct milestone contains the exact evidence
URL and description. Portal acceptance alone is insufficient. A timeout has an
unknown outcome: check that transaction ID before retrying. Leave withdrawals,
payout statuses and other milestones unchanged.
