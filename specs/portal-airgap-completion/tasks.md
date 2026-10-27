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
