# Implementation Plan: Private invoices

## Technical context
Rust workspace, existing reqwest/serde/chrono/cryptoxide/pallas dependencies. Add
`minos/src/invoice.rs` for typed portable packages, validation, COSE integrity,
private file I/O, stage receipts and portal operations. Add `invoice_cli.rs` for
Clap/dispatch; dispatch review/sign/verify before network client construction.
Reuse existing wallet signing only on explicit offline sign; keep evidence package
format unchanged. No new dependencies or helper executable.

## Constitution check
Preserve funds/witnesses: no chain operations. Evidence-derived claims: confirm
acknowledgement and authenticated list/download, never equate upload with attach.
Nix reproducibility: locked workspace build and all required checks. Consequential
behavior: mock HTTP plus offline cryptographic and CLI tests, synthetic files and
keys only. Failures are explicit; no automatic alternative or mutation fallback.
No constitution exception is needed.

## Protocol research (read-only, 2026-11-24)
Authoritative live schema: https://api.treasury.sundae.fi/graphql.
Observed client: https://treasury.sundae.fi/entry/index-Bmu7XcAK.js.
Public documentation: https://admin-services.docs.intersectmbo.org/guides/vendor-guides/2-delivery-and-milestone-management/how-to-submit-an-invoice.
Upload payload ordered fields: action=create_invoice_upload_ticket, instance,
project, milestone, sha256, invoiceNumber, invoiceDate, dueDate, amount, unit,
notes, nonce, timestamp. Session payload: action=unlock_invoice_session, instance,
project, nonce, timestamp. COSE_Key goes in publicKeyHex; COSE_Sign1 in signatureHex.
Upload ticket returns invoiceId/uploadUrl/expiresAt/confirmToken. Multipart POST
uses field `file`; returned data.cid or cid is passed to confirmInvoiceUpload.
The CID is private storage identity; no public-CID equivalence is assumed.
Confirm returns ok (schema scalar String). Session returns token/role/projectId/
expiresAt. List requires instanceId/projectId/sessionToken; download link requires
those plus milestoneId/invoiceId. API query errors propagate without downgrade.
The current UI requires invoiceNumber, invoiceDate, dueDate, amount and unit;
validates six decimal places, notes <=1000 chars, number <=120, dates ordered;
accepts PDF/PNG/JPEG <=10MiB. It labels unit lovelace as ADA but submits the decimal
ADA amount unchanged; implement explicit --unit string without guessing conversion
so contractual token units (including USDM) can be supplied exactly by operators.

## Data model and security
A versioned package contains network, signer key hash, optional file/basename and
exact serialized authorization bytes. Tagged payload variants distinguish upload
and unlock. Full package SHA-256 is additionally COSE-signed for integrity of
filename/network fields omitted from the portal's signature; the portal receives
only its expected authorization signature. Signed response binds the package hash.
Sensitive files are exclusive 0600, bounded reads; no public evidence metadata.
Online freshness uses portal time; offline verification trusts structural ordering
and signatures, not the local clock. CLI output reports target/hash/stage only.
Receipts persist ticket before upload, returned CID before confirmation, then
confirmed status. An explicit confirm command handles recovery; no automatic retry.
List/download use a versioned scoped session file, checked for expiry/project and
network, and verify file SHA-256 from authenticated list metadata before saving.

## Validation
Use the project Nix dev shell. Tests: exact payload golden serialization; file,
claims and signature tampering; wrong key/network; clock/freshness separation;
private files/no overwrite; multipart/response parsing; mock ticket/upload/confirm
success and recovery; mock unlock/list/download and integrity. CLI tests prove
offline dispatch with unreachable portal and no real keys. Run workspace tests,
fmt, strict Clippy, package builds and flake check. No live write smoke test.
