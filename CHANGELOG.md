# Changelog

## Unreleased

- Make the expected request hash optional for airgap signing while retaining
  explicit hash-pin validation and the signed response's package binding.

- Make offline airgap review/sign/verify independent of local clock drift while
  preserving online expiry and authorization-time checks.

- Restore Sundae portal defaults and detect its legacy read schema while retaining
  strict validation for the bundled API-v2 debugging portal.
- Verify UnixFS PDF CIDs offline so one airgap run signs both the transaction and
  CIP-30 upload authorization without a returned-CID mismatch.
- Preserve existing witnesses when measuring fees during signed preflight.

- Require explicit mnemonic display and create private wallet files without
  overwriting existing paths.
- Preserve transaction body and witness bytes when signing; reject unsupported
  CBOR shapes before writing output.
- Verify published wallet derivation, address and signature vectors.
- Preserve residual vendor value after the final scheduled payout and return
  native assets from wallet fee inputs as change.
- Configure staking-script reference outputs explicitly and verify their hashes.
- Reject unsupported combinations of reward scripts before transaction output.
- Calculate vendor-operation fees iteratively with exact prices, encoded witness
  sizes and tiered reference-script fees.
- Bind the portal to loopback by default and require explicit development auth
  bypass when no authorized registration keys are configured.
- Bound HTTP requests, response sizes and GraphQL work.
- Scope portal project, output, event and dedup identities by instance; version
  databases and reject incompatible network/schema settings.
- Include integration fixtures in package builds and embed DuckDB's JSON
  extension for offline operation.

The remaining implementation and release gates are tracked in
`specs/001-open-source-release/tasks.md`. This entry does not declare a release.

- Replace CLI atomic publication with two independently bounded unsigned
  transactions and an explicit dependency/hash manifest; show reference readiness
  separately from registry confirmation.
- Rehearse offline migration and restoration across process boundaries; remove
  temporary restore-marker trees when absent from the original backup.
- Patch TLS, Tokio and a yanked macro dependency; include complete Rust/Elm,
  bundled DuckDB and native runtime notice collections with source provenance.
