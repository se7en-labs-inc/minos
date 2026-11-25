# Feature Specification: Private invoice submission

**Feature Branch**: `003-private-invoices`
**Created**: 2026-11-24
**Status**: Accepted for implementation
**Input**: Add support for Intersect/Sundae's private invoice upload APIs in Minos.

## User Scenarios & Testing

### US1 — Attach an invoice with an offline wallet (P1)
A vendor prepares a private invoice request online, reviews/signs it on the airgap,
and returns the response for upload and confirmation on the selected milestone.
Given a supported invoice and complete claims, preparation embeds the exact file,
SHA-256 and fresh authorization without accessing a wallet or uploading anything.
Given the matching signed response, upload verifies its integrity before requesting
one private ticket, posting multipart bytes and confirming the returned private CID.
No public evidence mutation, Cardano transaction or public IPFS URL is produced.

### US2 — Recover after upload without uploading twice (P1)
Given upload succeeds but confirmation fails, a private receipt preserves the
invoice ID, target, confirmation token and returned CID. An explicit confirm
command retries that operation, without another wallet signature/ticket/upload.
A ticket and upload receipt are saved before the next external action. Unknown
outcomes are reported by stage; existing artifacts are never overwritten.

### US3 — Verify the attached invoice privately (P2)
Prepare/review/sign an invoice-session authorization offline, exchange it online
for a restricted session, list invoices, and download one into a private new file.
Expired/wrong-network/wrong-project sessions fail before queries or downloads.
Download verifies SHA-256 against invoice metadata. Session tokens and capability
URLs are stored only in private files and are never printed in normal output.

## Requirements
- FR-001 Use the observed Sundae GraphQL schema and exact JSON.stringify field
  ordering for invoice upload/session payloads; document the observed bundle.
- FR-002 Bind network, signer, target, claims and exact file to an independently
  verified offline request. Reuse CIP-30 COSE signing and GPG/account derivation.
  Protect ancillary package fields not covered by the portal payload as well.
- FR-003 Support PDF, PNG and JPEG up to 10 MiB; validate actual file signatures,
  basename, dates, invoice number, decimal amount (up to six places), unit and notes.
  Never guess amount/currency, read real keys during tests or submit a transaction.
- FR-004 Offline review/sign/verify use no network or local-clock gate. Online
  signature exchange enforces fresh portal time (two minutes, bounded future skew).
- FR-005 Private requests/responses/receipts/sessions/downloads are created
  exclusively with Unix mode 0600. Existing files/symlinks are refused before
  external mutations. Avoid sensitive tokens, invoice contents and URLs in errors.
- FR-006 Upload uses the private ticket only, requires HTTPS without credentials,
  disables redirects and bounds responses. Private CID is opaque: do not compare
  it to the public evidence UnixFS CID or expose it as public evidence.
- FR-007 Completion requires an acknowledged confirmInvoiceUpload; uploaded alone
  must not be reported as attached. Preserve recovery artifacts on failures.
- FR-008 Add CLI/docs/completions coverage, pure cryptographic/tamper tests and
  mock HTTP end-to-end tests; no production write integration is authorized here.

## Edge cases
Wrong key/account/network/action, changed file or claims, stale authorization,
invalid dates/amounts, unsupported file, HTTP/GraphQL failures, missing CID,
ambiguous confirmation, duplicate file destinations, invalid/expired session,
wrong invoice selection and changed download bytes all fail with stage context.

## Scope and assumptions
Implement a separate `minos invoice` command family. Invoice creation/accounting,
invoice deletion, finance-wide sessions, local portal schema changes and live
submission of a real invoice are outside scope. Vendor project sessions suffice.
The portal remains authoritative for role authorization and storage privacy.

## Success Criteria
- SC-001 A mock full upload ends with acknowledged confirmation using only the
  returned offline signatures, and sends the exact file/claims to the right target.
- SC-002 Tampering, key/network mismatch and stale signatures cause zero mutations.
- SC-003 Confirmation can recover from the saved upload receipt without reupload.
- SC-004 Authenticated mock list/download verifies content hash and private output.
- SC-005 Existing evidence/transaction tests and project Nix checks pass unchanged.
