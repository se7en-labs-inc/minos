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
