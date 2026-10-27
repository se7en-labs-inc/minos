# Tasks

This historical implementation record excludes live wallet associations and
transaction handoffs. Live signing, uploads and submissions require separate
explicit authorization; these task statuses do not authorize them.

- [x] T001 Implement Rust CID calculation and local-file CLI evidence.
- [x] T002 Implement verified portal fee selection and registry discovery.
- [x] T003 Implement explicit witnessed transaction submission through portal.
- [x] T004 Test CID, fee selection and portal request boundaries; document usage.
- [x] T005 Generate and evaluate an unsigned completion for the authorized handoff.
- [ ] T006 Verify detached witness, assemble without changing the body, recheck
  validity/inputs, submit through the authorized route and confirm chain inclusion.
  Historical live completion remains separate from offline implementation tests.
- [x] T007 Test strict detached witness decoding, signature/signer/body checks
  and preservation of body, metadata and existing script witnesses.
- [x] T008 Include vendor txAuthor in completion metadata; test author validity,
  required signer membership and auxiliary hash binding.
- [x] T009 Build and evaluate a replacement unsigned completion after author
  correction; require a fresh witness for its changed body.
- [x] T010 Define and validate a self-contained PDF/transaction/upload request
  and signed response, including body/metadata/file/context binding.
- [x] T011 Implement CIP-30 COSE signing and offline airgap review/sign/verify;
  test tampering and independent decoding with disposable keys.
- [x] T012 Implement online prepare-package/upload-package, request verification,
  CID guard and signed output; test mock ticket/upload boundaries.
- [x] T013 Build/test integrated Minos commands without loading production keys
  or performing uploads/submissions during tests.

Validation at implementation: 45 Minos tests passed. Coverage includes independent
COSE decoding/OpenSSL verification, disposable GPG-encrypted mnemonic I/O, package
binding/tampering/expiry/network/wrong-key rejection, multipart PDF/CID guards and
mock portal time/ticket contracts. Production mnemonic was never loaded by tests.

Completion excludes the unused treasury reference and includes verified staking
script authorization when required by the vendor permission. Offline regression
fixtures evaluate both the vendor and allowlist validators while preserving
vendor value, assets and datum. Metadata binds txAuthor to a required vendor key.

Remaining integration limits: live upload authorization freshness, import CID
behavior, sponsor signatures and actual chain inclusion must be checked for each
explicitly authorized operation. Portal registration is distinct from broadcast
and confirmation. Earlier transaction handoffs and expiry times are private
operational records, not reusable release instructions.

## Authorized full-package correction (2026-10-27)

- [x] T014 Restore public Sundae defaults and advertised-schema read compatibility with strict v2
  unchanged; test actual schema shapes and unknown confirmation semantics.
- [x] T015 Implement deterministic UnixFS CIDv1 evidence binding; validate against
  published vectors and independent Kubo chunk-boundary/multichunk results.
- [x] T016 Extend combined-package preparation/offline validation to supported
  metadata CIDs, retain raw format and exact upload match; test both signatures
  and tamper rejection. Document airgap binary update and import profile.
- [x] T017 Run meaningful checks and regenerate a fresh mainnet combined request
  with live state/protocol evaluation and a new nonce/server timestamp. Preserve
  old payloads and all local wallet/transaction/report files; do not sign/submit.

T014 clarification: restore Sundae network defaults and negotiate advertised read
schema, rather than requiring a legacy flag. Re-run the combined-signature live
workflow using the existing whitelisted preview wallet and instance. No new
instance, disposable key or unrelated project is authorized for that rehearsal.

Correction validation: all 152 workspace Rust tests, strict workspace all-target
Clippy and formatting pass. Kubo 0.40.1 independently reproduces the UnixFS CID at
empty, exact-chunk, over-chunk, multichunk and 10-MiB limits. Signed preflight
preserves existing witnesses and adds only missing size placeholders. The full
preview run used the existing whitelisted wallet and instance, returned both
signatures, uploaded with an exact CID match, passed signed preflight and confirmed
on chain; the maintainer confirmed the milestone evidence in Sundae's UI. No new
instance was deployed. A fresh mainnet combined request passes both validators;
mainnet signing and submission remain separate airgap operations.

- [x] T018 Remove local-clock gating from offline review/sign/verify while retaining
  all structural/cryptographic checks and encoded validity display. Keep online
  time enforcement, test +/-one-day drift and malformed windows/timestamps, and
  validate the existing payload unchanged with the updated binary.

- [x] T019 Make --expected-request-hash optional while preserving explicit mismatch
  rejection before key access, package review and returned-response hash binding.
  Verify the normal signing command needs no manually copied hash.

Clock/UX validation: all 153 workspace tests, strict all-target Clippy and formatting
pass. Independent libfaketime runs confirm the actual offline CLI reviews the same
payload with +/-one-day drift and verifies returned signatures with a shifted
clock; online upload rejects the expired transaction before creating output or
contacting the portal. Structural window/timestamp errors still fail offline.
Encrypted disposable-wallet CLI signing succeeds with no expected-hash option;
an explicitly wrong hash fails before accessing a nonexistent wallet. Existing
real package bytes and request hash remain unchanged.
