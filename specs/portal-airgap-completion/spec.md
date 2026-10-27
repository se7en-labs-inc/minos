# Portal completion with an airgap witness

The operator needs to prepare milestone completion evidence from a
local signed PDF, witness the transaction on an airgap, and return it for portal
submission. The user explicitly requests local CID calculation in Rust and no
file upload, and selects the portal's sponsored fee inputs.

## Requirements

Support operator-funded fees; sponsored evidence fees may require portal review. PDF upload
still uses the portal; the unified request must require only the vendor key.

- A single self-contained airgap request must bind the exact transaction body,
  PDF bytes, evidence CID, vendor signer and portal upload authorization. One
  offline signing run returns both a TxWitness and CIP-30 COSE message signature.
  The signer must show a review, require explicit signing intent, use no network,
  and never export private keys. Verify the returned artifacts before use.
- Preparation uses fresh portal time and a random nonce. Preserve exact UTF-8
  payload bytes; the upload request uses the same instance/project/milestone and
  file details. Do not sign expired transactions or silently refresh payloads.
- Online upload consumes the returned signatures, verifies the PDF and request,
  uses the ticket URL, and compares the upload CID to the signed evidence URL
  before producing a ready transaction. A CID mismatch requires another witness;
  one-pass signing cannot remove that dependency if import settings differ.
- The airgap produces signatures only for keys supplied by the user; the portal
  fee wallet witness remains an external requirement. No upload/submission is
  performed while implementing or testing the tool.

- Calculate an explicit CIDv1 raw/SHA-256/base32 identifier from file bytes in
  Rust. Print the IPFS URL and make local files usable as completion evidence.
  Calculation never uploads, pins, or establishes availability. State the codec;
  UnixFS imports with different settings need not produce the same CID.
- Build an unsigned completion, preserving vendor value and datum, using only
  live, unreserved, ADA-only portal fee inputs and returning change to their
  address. Never load a signing key. Keep personal fee funding available.
- Discover the registry NFT independently of deployed reference-script outputs;
  validate references. Preserve all payouts during an evidence-only completion:
  use a validity lower bound before the first active payout matures. The real
  validator permits maturation inside the interval; require local evaluation.
- Provide explicit portal submission via newTransaction for the returned
  witnessed transaction. Never fall back from registration-only mode to that
  mutation. Report portal acceptance separately from chain confirmation.
- Stop after producing the unsigned completion transaction and review details. Submit
  only when the user returns the witnessed transaction.
- Include a vendor required signer as `txAuthor` in completion metadata. Reject
  missing or malformed vendor author keys before building. An author correction
  requires a fresh body hash and airgap witness; preserve earlier artifacts.

## Acceptance

Returned detached `TxWitness ConwayEra` envelopes must be strictly decoded,
verified over the expected original body hash, and matched to the required
vendor signer. Assembly must preserve the original body, metadata and script
witnesses. Reject malformed, trailing, wrong-signer and invalid-signature data.
Recheck validity and live inputs before the authorized portal submission.

Known CID vectors and file/byte consistency pass without a network. Portal fee
selection excludes reserved, spent and asset-bearing rows. A mock portal verifies
submission uses newTransaction and preserves exact CBOR. The actual unsigned
transaction contains the selected PDF URL, preserves vendor output, and passes
local script evaluation before handoff. No upload or submission is performed.

## Authorized full-package compatibility correction (2026-10-27)

The complete offline operation must return both the transaction witness and CIP-30
COSE upload signature in one signing run. Restore Sundae as the network default
and detect its advertised read schema; endpoints advertising API-v2 still require
strict version and confirmation validation, without error-triggered downgrade.
Missing legacy confirmation fields must remain unknown rather than fabricated.
Transaction construction must still resolve live inputs/registry/scripts and pass
current preflight before writing an unsigned payload.

Bind the package's evidence URL to the PDF bytes using either the existing raw CID
or a deterministic UnixFS CIDv1 import: SHA-256, 256-KiB fixed chunks, raw leaves,
balanced DAG, no directory wrapping/mode/mtime. Select the representation from the
signed metadata and independently recompute it offline. Reject any other URL or
changed bytes. Keep exact CID comparison after upload. Never insert a returned
CID into an already signed transaction. Preserve existing package format/raw support.

## Offline clock independence (2026-10-27)

Offline review, signing and signature verification must not trust or require the
machine's wall clock. Validate package/body/metadata/PDF/CID/network/signer and
well-formed validity/timestamp relationships; display the encoded validity window.
A day of clock drift in either direction must not prevent review or signing.
Online preparation, upload and transaction preflight/submission retain current-time
checks. Signing does not extend a transaction's chain validity or mutate the
server-issued upload timestamp. Preserve existing payload bytes and request hashes.

The expected-request-hash signing flag is optional. Always display the package
review before loading the wallet, and retain its hash binding in the returned
signed response. An explicitly supplied hash must still match before key access.
