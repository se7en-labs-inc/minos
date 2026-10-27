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
