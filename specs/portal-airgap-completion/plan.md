# Plan

## Unified airgap package (2026-10-26)

User requests one package and one signing run covering transaction and PDF
upload. Extend the existing spec/worktree. Add typed package/request/response
models and validation in minos/src/airgap.rs, CIP-30 COSE signing in
minos/src/cose.rs, and minos airgap review/sign/verify in the existing binary.
The user deploys Minos themselves; do not create a separate executable or
deployment archive. Reuse the existing GPG-encrypted mnemonic wallet support.
Dispatch offline commands before constructing network clients. Require expected signer,
exact body/auxiliary hash, file bytes/hash/CID, metadata context and transaction
validity checks. Bundle hash binds signatures to the complete request. Include
the PDF bytes so the airgap requires only one input file.

Online minos evidence prepare-package queries portal time, generates a 16-byte
nonce and constructs the exact observed JSON authorization. upload-package
verifies all return signatures, obtains createEvidenceUploadTicket, uploads
multipart file to its HTTPS URL, checks CID and writes a vendor-witnessed
transaction without submitting. Do not assume an automatic sponsor signature.
Use CIP-8/30 primary specs; test COSE structure and Ed25519 with independent
decoding and known published vectors where available, tampering, wrong signer,
file/context/hash/expiry mismatches and mock ticket/upload HTTP boundaries.
Build/test the integrated Minos binary through Nix. The earlier sponsored transaction is already
registered; do not resubmit it during development. Portal authorization freshness
and actual Pinata CID behavior remain live integration limits until authorized
signatures return. Do not promise a fixed airgap authorization lifetime.

## Approved author correction (2026-10-26)

User approved fixing the generator and preparing a replacement after the remote
portal rejected an empty transaction author. Pass the first vendor required key
(validated as 28 bytes) to encode_complete and emit txAuthor using the existing
metadata encoder. Do not use the fee sponsor as author. Test decoded metadata,
required signer membership and auxiliary hash binding, including rejection of
empty/malformed author keys. Regenerate under a new filename using current live
sponsored inputs, recalculate fee and evaluate before handoff. Preserve the
previous transaction and witness; the replacement requires a fresh witness.

Expose `assemble --tx --witness --expected-body-hash --required-signer --out`.
Verify the detached Ed25519 witness in tx::sign before attaching it with the
existing byte-preserving helper; decode Conway before and after and check the
body hash. Require the signer in the transaction's required_signers. Test
preservation and rejection paths, then run mainnet preflight before submission.

Continue in the existing working tree with its pending MAF and backend changes;
use an unnumbered spec consistent with the existing local feature artifacts.
No branch allocation or commit is needed.

Use existing cryptoxide SHA-256 and a small base32 encoder, without dependency
changes. Add evidence cid and complete --evidence-file. Add complete --portal-fees
as an explicit alternative to --fee-addr. Query portal feeInputs, filter by
reservation, verify against chain UTxOs and use actual chain values. Resolve the
REGISTRY token through asset_utxos and validate all reference scripts.

Add submit --via-portal, mutually exclusive with --portal-only. The explicit new
mode invokes the live portal's newTransaction mutation, without local submission
or registerTransaction. Retain current modes. Verify Conway transaction vkey
witnesses before sending. Document raw CID limitations and no availability claim.

Run targeted Rust tests, mock portal contract tests, formatting and cargo check
through Nix. Build the real mainnet transaction without keys, decode the body and
metadata for review, and run the existing local evaluator. Do not rebuild after
the user witnesses it; verify its body hash when returned.

## Approved completion correction

The initial real transaction failed UPLC evaluation because it referenced the
unused treasury script. The user approved removing it and regenerating on
2026-10-26. Complete executes the vendor Withdraw redeemer only, so reuse
withdrawal_references and remove treasury fields from CompleteParams. Test that
the builder includes exactly the registry and vendor references while preserving
the full vendor output and datum. Recalculate the fee from those two references.

## Approved allowlist authorization correction

The user approved adding the script permission required alongside the vendor
signature. Share the existing known staking-reference lookup with withdrawal.
Include its verified reference input, zero reward withdrawal and reward redeemer
using add_treasury_withdrawal; include its execution and reference-script costs
in the fee. Preserve the vendor output. Run preflight inside cmd_complete before
writing the unsigned file so a failed transaction is never handed off as ready.
Add an offline fixture test that exercises both vendor and allowlist validators.

## Full-package compatibility correction

Detect the advertised portal read schema and use schema-specific queries. Endpoints
advertising v2 must reject missing/unsupported versions and unconfirmed registrations.
Legacy instances expose unknown confirmation; live construction verifies chain
references independently. Do not add automatic mutation fallback or submission.

Implement bounded UnixFS CID construction in the existing Rust evidence module,
using existing SHA-256/base32 support and canonical DAG-PB encoding from the IPFS
UnixFS and IPLD DAG-PB primary specifications. Check known public vectors and
independent Kubo output at empty/single/chunk-boundary/multichunk sizes. Package
prepare/validate derives the expected metadata URL from exact PDF bytes, accepts
only supported representations, and reports the chosen import profile for review.
Test both returned signatures plus wrong-CID/content/profile tampering. Regenerate
current unsigned transaction and combined package under new filenames only after
live input/protocol checks. No mainnet keys, uploads or submissions in this work.

### Portal role clarification (2026-10-27)

Sundae's public mainnet/preview portals are the normal service; this repository's
portal is a debugging tool. Restore the public network defaults. Negotiate read
schema through explicit introspection once per client, with no error-triggered
fallback: absent apiVersion selects legacy only when required query fields are
advertised; advertised v2 requires version2 and confirmation fields. Missing
legacy confirmation remains unknown. Preserve explicit --portal for debugging.
The preview rehearsal uses the already configured whitelisted wallet and Base
instance; do not deploy another instance or use newly generated wallets.

Signed preflight must preserve/count existing vkey witnesses and synthesize only
missing ones for size estimation. The live combined-flow rehearsal exposed the
unsigned-only helper; cover partially/fully/extra-witness cases without changing
body bytes. Complete --evidence-file uses the same bounded UnixFS profile by
default; raw CID remains available explicitly through evidence cid/--evidence-url.

Split structural/cryptographic package validation from online current-time policy.
Offline CLI commands use only structural/cryptographic methods; online methods
retain strict expiry/not-yet-valid/future-authorization checks. Display absolute
encoded times without a misleading age computed from the offline clock. Add
positive skewed-clock signing/verification regressions and online rejection tests,
including malformed validity windows and authorization timestamps. Existing real
payloads and their hashes must not be rewritten to accommodate the clock.
