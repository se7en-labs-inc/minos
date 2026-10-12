# Implementation plan

Use pinned pallas-network 1.4.0, already present in the workspace lockfile.
Replace CardanoCli with LocalNode and asynchronous local-state queries.
Keep the DataSource interface so transaction builders do not change semantics.
Convert typed Pallas protocol parameters to the existing ProtocolParams type,
requiring fee, collateral, cost-model and execution-price fields explicitly.
Decode both legacy/current outputs, retaining Plutus reference bytes and inline
datum CBOR. Validate output references before connecting.

Use an owned client guard and explicit abort after release/done, including query
errors. On cancellation, schedule cleanup through the active Tokio runtime.
Derive time through Pallas network genesis values rather than SystemTime::now.
Use local transaction submission for signed Conway CBOR and hash the original
body bytes. Remove subprocess discovery, store paths and --cardano-cli.

Validation: pure conversion fixtures, Pallas mock Unix socket protocol integration
for queries/submission and cleanup, workspace tests, CLI help, no-subprocess scan.
No new dependency version resolution is needed; preserve the existing pins.

Scope excludes Plutus evaluation, withdrawal contract fixes, portal authorization,
and general warning cleanup. Those must not be implied by a successful build.

## Review follow-up plan

The exclusions above applied to the initial cleanup; the user has now authorized
these integration fixes and withdrawal preflight. Continue in the existing spec.
Expose reusable guarded query sessions from minos, with bounded operations and
cleanup, and replace the portal backend with a conversion adapter. Verify vkey
witnesses with pinned Pallas crypto. Make newTransaction a registration-only
mutation and use an atomic body-hash insert to preserve the original record.
Centralize strict reference-script resolution and validation. Use the pinned
uplc evaluator via CBOR to bridge its older Pallas types; preserve resolved output
CBOR and fail closed on unsupported evaluation data. Evaluate the final withdrawal
against fetched protocol parameters without requiring signing keys. Add tests at
these integration boundaries and document precisely what preflight does not prove.

Remove treasury reference fields from WithdrawParams and withdrawal CLI discovery.
Share strict reference resolution with an optional treasury reference internally;
retain the existing treasury resolver for other workflows. Test the withdrawal
builder and preflight with public fixtures; leave UPLC phase-one checks enabled.
