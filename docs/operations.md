# Portal operation and recovery

The bundled portal is for debugging. Normal CLI operations use Sundae's public
portal for the selected network. To use this local portal, pass its GraphQL URL
explicitly with `--portal`; the CLI supports both advertised read schemas.

Use the pinned Nix environment (`nix develop`). The portal embeds DuckDB; it needs
no database service. Its JSON extension is compiled in. The package supplies the
C++ runtime; the development shell is not a runtime dependency.

Copy `minos-portal/minos-portal.example.toml` to a private configuration file and
set the network, data path, allowed author hashes and backend. The default bind is
loopback. Empty authorization is refused unless the explicit local development
bypass is enabled. A registered transaction proves authorship, not confirmation.
No registration mutation signs or submits transactions.

Run `minos-portal portal.toml`. A node socket supports fresh replay from origin
without Koios or a pre-existing publish transaction. Instances registered while
running are discovered by the supervisor. Node connections have deadlines and
reconnect from a durable whole-block checkpoint; stale intersections fall back
to origin. On Ctrl-C, the HTTP listener drains and background tasks are aborted.

A Koios-only deployment discovers the registry, verifies the raw publication,
retrieves payment-credential transaction history (including spent outputs and all
stake-address variants), retrieves every page, and applies the same reducer as
node sync. CBOR body hashes and transaction/block positions must agree. A changing
tip, malformed response, missing transaction or failed page retains the previous
committed projection and retries. Large histories may require a node backend;
Koios replay currently requires an unchanged tip across its complete fetch.
Mixed deployments use the node for canonical replay and live output observations.
Koios is an upstream trust boundary, not a consensus proof.

## State and API compatibility

The current database schema is **3** and belongs to one network. Unversioned,
older, newer or mismatched databases are refused before serving. Sled contains
local registrations, tracked instances, the authoritative raw relevant-transaction
journal, canonical state/events and whole-block checkpoints. DuckDB is a
rebuildable event-query cache. A cache write failure cannot erase the Sled commit;
queries recover the cache from the authoritative event records.

GraphQL `apiVersion` is **2**. Deploy CLI, portal and UI together. An instance has
an explicit `confirmed` flag; pending local registrations do not establish projects
or chain events. Rejected or ambiguous claims appear in `observations`.
`Unresolved` means the portal cannot establish milestone lineage; `Unavailable`
means a tracked output could not be confirmed by the latest observation. Neither
means withdrawn. Only the verified contract transition establishes withdrawal.
An unavailable output is omitted from `unspentOutput` to prevent building against
a known stale snapshot. Asset quantities remain decimal strings, including u64
values that JavaScript cannot represent exactly.

## Offline backup and migration

Stop the portal before maintenance. The exclusive Sled lock refuses concurrent
portal access. Never run maintenance against the only copy without retaining the
original directory and its adjacent `-events.duckdb` and WAL file.

```
minos-portal backup ./portal-db ./portal-backup.json
minos-portal migrate ./portal-db ./portal-db-v3 preview
```

`backup` creates a portable hexadecimal JSON export of every Sled tree, with mode
0600 on Unix, and refuses an existing destination. For a legacy database, also
copy the exact adjacent DuckDB and optional WAL files while the portal is stopped;
the legacy event table is not an authoritative journal and is not included in the
Sled export. Preserve backups with restrictive permissions.

`migrate` copies local registrations, tracked instances and pending bootstrap
records into a **separate** database. It never modifies the original. It retains
a `source-backup.json` export in the destination, clears confirmation/reference
claims from bootstrap records and deliberately rebuilds projects/events/outputs
through chain replay. Network mismatches and unsupported source schemas fail.
Update `db_path` only after reviewing the new destination. Keep the original and
its adjacent DuckDB files until replay and application checks are complete.

An interrupted migration is refused by normal startup. Rerun the identical
`migrate` command to resume; the source snapshot and network must still agree.
A completed/existing destination is refused. For restoring a Sled export:

```
minos-portal restore ./portal-backup.json ./restored-db
```

The restore destination must be new or an interrupted restore of that same export.
Rerun the same command after interruption. A restored older schema still needs
migration into a separate database. DuckDB cache recovery for schema 3 is automatic
when queried; preserve legacy DuckDB files separately for historical inspection.

## Reindex and rollback

```
minos-portal reset-sync portal.toml
minos-portal reset-sync portal.toml --instance INSTANCE_POLICY_ID
```

Reset covers configured and dynamically tracked instances. It retains local
registrations and bootstrap records, clears the chosen instance's canonical chain
state/checkpoint, and replays history on restart. Other instances are unaffected.
An interrupted legacy-projection reset resumes on startup. A chain rollback,
including origin, atomically rebuilds the chosen instance from its retained raw
journal and replaces its events, current outputs and deduplication state.

Before accepting migration or reindex, compare confirmed publications, project
identities, mapped payouts, full native-asset quantities and rejection observations
against the source chain. Investigate unresolved lineage rather than assigning
milestones by payout count or lexical identifier order.

## UI deployment

Serve the `minos-ui` package's directory over HTTP(S). Edit/copy `config.js` to set
`window.MINOS_PORTAL_URL`; the default `/graphql` expects a same-origin reverse
proxy. For separate origins use an absolute portal URL and configure the proxy's
access controls. Serve the UI and API through HTTPS when crossing an untrusted
network. The browser request deadline is 30 seconds. A pending badge and explicit
unresolved/unavailable statuses are part of the API v2 contract.
# Registration acknowledgements

GraphQL API version 2's `indexTransaction` verifies the fetched transaction's
body hash, metadata and allowed witness, then durably queues its instance for
the supervisor. `ok: true` acknowledges tracking. It does not mean an event is
confirmed. Query `confirmed` and project availability after background replay.
History replay uses the same validation and journal as automatic ingestion.

## Two-stage publication readiness

The CLI's deployment directory contains `publish.json`, `vendor-reference.json`
and `manifest.json`. Confirm the first transaction before submitting the second;
its only spending input is the first transaction's change at index 2. The registry
and treasury reference are first-stage outputs 0 and 1. Vendor reference is the
second transaction's output 0. Use the manifest or advertised outrefs, not a
legacy `publish-tx#2` vendor-reference assumption.

GraphQL `confirmed` means the registry publication is confirmed. `referencesReady`
is true only after registry, treasury and vendor references are known/current.
The UI shows references pending between stages. Rollback or consumption clears
reference readiness. Preserve the public bundle for review and recovery; automatic
rebuilding of an expired second-stage artifact is outside the CLI scope.
