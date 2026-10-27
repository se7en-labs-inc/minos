# One airgap round trip using Minos

Install updated Minos on the airgap using your usual deployment process. All
functionality is in the existing `minos` executable. It reuses the same
GPG-encrypted mnemonic wallet as `minos wallet sign`, account 0 by default.
The `minos airgap` commands run without contacting the portal or a node.

## Prepare online

Build an unsigned completion using `complete --fee-addr <your-mainnet-address>`
and the signed PDF as `--evidence-file`, then:

```bash
minos --network mainnet evidence prepare-package \
  --tx complete-unsigned.json \
  --file evidence.pdf \
  --out completion-airgap-request.json
```

The request embeds the exact transaction, PDF bytes and upload authorization.
Preparation reads portal server time and generates a random nonce; it does not
load keys, upload or submit. Prepare shortly before transfer/signing: the portal
may enforce a freshness window shorter than the displayed transaction expiry.
Its exact upload authorization lifetime remains unverified.

Evidence transaction fees use your own funded address. Sponsored fee inputs
may require review by the portal operator. PDF uploads still use the
portal's upload API.

## Review and sign on the airgap

Transfer only the request to the machine where you installed Minos:

```bash
minos airgap review --package completion-airgap-request.json
minos airgap sign --package completion-airgap-request.json \
  --wallet /path/to/wallet.asc \
  --expected-request-hash <requestHash-from-review> \
  --out completion-airgap-signed.json
minos airgap verify --package completion-airgap-request.json --signed completion-airgap-signed.json
```

Review network, signer, fee, inputs/collateral, outputs, expiry, PDF hash,
metadata and upload message. One signing run returns the transaction witness
and CIP-30 COSE upload authorization together. GPG uses your offline keyring.
Only return `completion-airgap-signed.json`; it contains no private keys or mnemonic.
The commands reject wrong keys, expired transactions, changed PDF/context and
invalid signatures, and refuse to overwrite an existing signed response.

## Upload online and finish

```bash
minos --network mainnet evidence upload-package \
  --package completion-airgap-request.json --signed completion-airgap-signed.json \
  --receipt completion-upload-receipt.json --out completion-ready-signed.json
```

This verifies both signatures before requesting the portal upload ticket,
uploads the embedded PDF to its HTTPS URL, records the returned CID, and writes
the assembled transaction only when that CID matches the signed evidence URL.
A different CID requires a new transaction/witness; the tool saves the actual CID
in a receipt and stops. It never silently replaces the signed body.

Upload does not register or broadcast the transaction. Run mainnet `preflight`
and recheck current validity/live inputs before chain submission. Self-funded
evidence should be directly submitted with all required signatures. Portal
`newTransaction` registration only collects signatures; confirm actual chain
inclusion and then confirm evidence appears on the milestone.
