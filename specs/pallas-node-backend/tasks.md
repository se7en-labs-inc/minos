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
