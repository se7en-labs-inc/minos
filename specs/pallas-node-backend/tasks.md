# Tasks

- [x] T001 Review existing CLI and portal backends and Pallas 1.4.0 APIs.
- [x] T002 Replace subprocess queries with typed local-state queries and cleanup.
- [x] T003 Add native transaction submission and remove CLI binary configuration.
- [x] T004 Test conversion fidelity, invalid inputs and protocol lifecycle locally.
- [x] T005 Update documentation and run Nix workspace validation.
- [x] T006 Review and commit the authorized cleanup with pending code checkpoints.

Validation: workspace tests pass (13 tests in each minos target, including 10 new
backend tests); Rust formatting and CLI build/help pass. Nix UI build passes and
the Rust package derivation evaluates. The extracted core commit also passes
`cargo check --locked --offline` independently of the MAF command. No live node test or chain submission was
performed. Existing unrelated compiler warnings remain.

Test fixture corrections: Pallas byte wrappers require slice/nonempty-map
conversion. Pallas 1.4.0's rejection encoder does not round-trip its decoder, so
the mock submission server emits the ledger's outer-array rejection shape through
a generic Pallas server. Production uses the standard Pallas client decoder.

## Review follow-up

- [x] R001 Share guarded node sessions between CLI and portal; add operation deadlines.
- [x] R002 Verify portal author witnesses and separate idempotent registration from submission.
- [x] R003 Require complete reference-script queries and validated script hashes for fees.
- [x] R004 Add local withdrawal preflight and realistic offline regression fixtures.
- [x] R005 Run Nix workspace validation, update docs and commit fixes by concern.
- [x] R006 Remove the unused treasury reference from vendor withdrawals and fees;
  verify the builder and real-script preflight regression.

R004/R006: the user authorized removing the unused treasury reference. New vendor
withdrawals omit it from discovery, transaction construction and script-byte fee
accounting. The real fixture passes both validators with only that reference
removed; redirecting its payout to the fee payer fails. UPLC phase-one checks stay
enabled. Builder and strict resolver regressions pass. All 28 workspace tests pass
through the Nix dev shell. No signing or submission was performed.

R005 status: portal, node and strict reference fixes were committed independently
as 1001807, 68c0c45 and 5f2d2e0. Final withdrawal validation and documentation are
committed together. Nix workspace build and all 28 tests pass; formatting and
diff checks pass. Existing unrelated compiler warnings remain.
