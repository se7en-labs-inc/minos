# Minos

Minos provides Rust tooling for the SundaeSwap treasury contracts on Cardano:

| Component | Purpose |
|---|---|
| `minos` | Build, inspect and sign transactions; prepare evidence and MAF documents |
| `minos-portal` | Validate treasury history, maintain confirmed projections and expose GraphQL |
| `minos-ui` | Elm dashboard for the portal |

The open-source release is in progress. Publication uses two bounded unsigned
transactions with an explicit confirmation dependency and deployment manifest.
Consult the [acceptance checklist](specs/001-open-source-release/checklists/release.md),
[support matrix](docs/support-matrix.md), [operations guide](docs/operations.md)
and [release procedure](docs/releasing.md) for current evidence and remaining gates.

## Build and check

Use Nix with flakes enabled. The development shell provides the pinned Rust/Elm
build tools and GPG. A Cardano node socket or a Koios endpoint supplies chain data;
no `cardano-cli` executable or external database service is required.

```sh
nix develop
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- --deny warnings
cargo fmt --all -- --check
nix build .#minos .#minos-ui
nix flake check
```

`nix fmt` applies Rust, Elm, TOML and Nix formatting. The `minos` package installs
both Rust binaries, includes integration fixtures in its checks, supplies the
portal's C++ runtime and wraps the CLI with GPG. The UI package runs its Elm worker
tests and installs the static application. [Contributing](CONTRIBUTING.md) explains
the development workflow; [SECURITY.md](SECURITY.md) covers private reporting.

## CLI configuration

```sh
export CARDANO_NODE_SOCKET_PATH=/run/cardano/preview.socket
minos --network preview --portal http://127.0.0.1:8080/graphql instances
minos --help
```

| Flag | Behavior |
|---|---|
| `--network preview` / `--network mainnet` | Select addresses, network magic, time conversion and Koios defaults |
| `--node-socket PATH` | Query and submit directly through Pallas; also accepts `CARDANO_NODE_SOCKET_PATH` |
| `--node-timeout-secs N` | Per connection/query deadline, default 30 seconds |
| `--koios-key KEY` | Optional Koios API key; also accepts `KOIOS_API_KEY` |
| `--portal URL` | Defaults to the network's Sundae portal; set explicitly for a local debugging portal |
| `--script-reference HASH=TX_HASH#INDEX` | Repeat for verified reference-script locations |
| `--vendor-signer HASH` | Repeat to select vendor threshold signers or a script-only metadata actor; alias `--signer` |

Without a node socket, the CLI uses the selected network's Koios API. Historical
transaction fetch requires Koios. Node asset-by-policy discovery is intentionally
unsupported: provide a filtered reference outref or use Koios. The portal exposes
registry and script outrefs so references may reside in separate transactions.
A network-magic override does not add another CLI network's time/HTTP configuration;
CLI network choices are preview and mainnet. The portal also supports preprod.

## Wallets and signing

```sh
# Save an encrypted wallet; only the output path is printed.
minos wallet new --gpg-recipient you@example.com --out wallet.asc

# Displaying a new mnemonic requires this explicit flag.
minos wallet new --show-mnemonic

minos wallet address --wallet wallet.asc
minos wallet address --wallet wallet.asc --account 1 --base
minos wallet inspect --tx unsigned.json
minos wallet sign --wallet wallet.asc --tx unsigned.json --out signed.json
```

Wallet derivation uses BIP-39/Icarus. Saved wallet files use owner-only permissions
and exclusive creation; existing files and symlinks are refused. `--show-mnemonic`
is required to print the phrase. GPG must have the selected recipient key available.
Treat mnemonic files and decrypted output as secrets and keep them outside Git.

Each wallet supplies one signature. For multiple signers, sign sequentially into
new files, supplying each previous signed file as input. Earlier witnesses and the
original body/auxiliary bytes are preserved. Accounts from one mnemonic represent
separate roles but share the same underlying seed. Inspect the complete transaction
and expected body hash before signing. [Airgap completion](specs/portal-airgap-completion/airgap-quickstart.md)
describes detached evidence signatures and assembly.

Building and signing do not submit. Explicit submission is a separate operation:

```sh
minos --network preview wallet submit --tx signed.json
```

A timeout during submission is an unknown outcome with a body hash, not a rejection;
check the chain before retrying. Successful submission still requires confirmation.

## Treasury lifecycle

Use `minos COMMAND --help` for current required arguments and output options.
Builders write unsigned transaction JSON and run applicable local validation before
writing. They preserve native assets, calculate protocol minima/fees and require
separate ADA-only collateral for Plutus transactions. Local evaluation is not full
ledger acceptance; input availability and protocol parameters may change.

| Command | Workflow |
|---|---|
| `publish` | Two-stage registry NFT and reference-script deployment bundle |
| `mint` | Single-signature native test token policy, optional CIP-68 reference NFT |
| `deposit-project` | Wallet deposit for a project template |
| `fund-project` | Fund a portal project template from the treasury |
| `fund` | Construct a project/vendor output from milestone inputs |
| `complete` | Preserve payouts/value and attach authenticated completion evidence |
| `withdraw` | Claim matured active payouts to the selected destination |
| `adjudicate` | Pause/resume payouts under governance authorization |
| `modify` | Correct the vendor datum with governance and existing vendor consent |
| `status`, `instances` | Read portal state; pending/unavailable data is explicit |
| `preflight` | Resolve inputs and locally validate a prepared transaction |

`publish -o NEW_DIRECTORY` creates `publish.json`, `vendor-reference.json` and
`manifest.json`. Without `-o`, stdout contains a JSON bundle rather than a single
transaction. Inspect both transactions and the manifest's body hashes. Sign and
submit `publish.json`, wait for its confirmation, then sign and submit
`vendor-reference.json`. The second transaction spends the first transaction's
exact change output. Nothing is signed or submitted automatically. The manifest
is written last; a directory without it is incomplete. Existing directories are refused.
The vendor-reference transaction shares the initial validity window. If it expires
after registry confirmation, retain the public deployment artifacts and prepare a
replacement reference transaction; automatic rebuilding of an expired stage is
outside the current CLI scope. Do not attempt to republish with the consumed seed.

For threshold administrators, repeat `publish --admin-key HASH` and select
`--admin-threshold N`. The first key authors the metadata. Funding's `--co-signer`
adds administrators; `--vendor-key` identifies a separate vendor. Vendor operations
can select a sufficient subset with repeated `--vendor-signer`. Script-only vendors
need an explicit metadata actor and the matching configured reward-script reference.
Only one distinct reward script per transaction is supported.

References are verified against the registry and script hashes. Legacy
`--publish-tx` hints retain their explicit positional convention; normal discovery
uses advertised/configured outrefs. An unavailable or incomplete reference fails
before transaction output. Withdrawal preserves paused/future payouts and every
residual asset even after the last scheduled payout. A missing output alone does
not establish a withdrawal.

The bundled portal registers witnessed transactions for tracking through
`registerTransaction` / its `newTransaction` alias. Registration never signs,
sponsors or submits. `indexTransaction` verifies a fetched transaction and durably
queues background history replay; its acknowledgement does not mean confirmation.
The CLI normally uses Sundae's public portal for the selected network. It detects
the advertised read schema; API-v2 endpoints retain strict version/confirmation
checks. Legacy Sundae confirmation is unknown until checked on chain. The bundled
API-v2 portal/UI are debugging tools; select them explicitly with `--portal`.
Sponsored fee signing is a separate operator-controlled workflow.

## Portal and UI

```sh
cp minos-portal/minos-portal.example.toml minos-portal.toml
# Edit the socket/backend, network, instance IDs and allowed author key hashes.
minos-portal minos-portal.toml
```

The portal binds to loopback by default. Configure allowed 28-byte author key hashes.
An empty list requires explicit `development_auth_bypass = true` with a loopback
bind; production must use authorized authors. Configure `node_socket_path`,
`koios_url`, or both. Known network URLs and instance networks must match the
configured network magic. See the example file for all settings.

Node mode replays from origin or a durable checkpoint, recovers stale intersections
and follows rollbacks. Koios-only mode obtains complete paginated history before
atomically replacing projections; inconsistent/incomplete history retains the
committed state. Dynamically registered instances start automatically. Snapshots
report output availability while validated chain transitions establish events.

Sled schema 3 is authoritative for the journal, state/events and checkpoints.
DuckDB is the embedded rebuildable query cache, with JSON support bundled; it
requires no service or extension download. Do not open an older database as if it
were current. [Operations](docs/operations.md) documents offline backup, restore,
restartable migration, reindex/reset, rollback and deployment. Maintenance rehearsal
runs operations across process boundaries like the offline CLI; backup, restore,
source preservation and interrupted migration resume are verified.

The GraphQL endpoint is `/graphql`; query `apiVersion` to verify version 2. Pending,
confirmed, unresolved and unavailable states are distinct. The UI is a static Nix
output: serve its `index.html`, `main.js` and `config.js`. Configure
`window.MINOS_PORTAL_URL` in `config.js`, or proxy same-origin `/graphql` (the default).
Keep CLI, portal and UI on the same API version.

## Evidence and MAF documents

`minos evidence cid --help` calculates an evidence CID locally without uploading or
pinning. `evidence prepare-package` / `upload-package` prepare and perform explicit
upload workflows; uploading does not confirm an on-chain completion.

`minos maf github --help` generates a DOCX Milestone Acceptance Form from a project
configuration, date range, merged PRs and releases. `minos maf report --help`
generates one from a report. GitHub fetching paginates PRs/releases, bounds responses
and reports incomplete upstream results. The project example in
`minos/tests/fixtures/maf/project.md` uses synthetic signatories. Review generated
content and evidence before any signature or publication.

## Provenance and release review

[Embedded contracts](docs/contracts.md) records the exact upstream revision,
compiler/configuration, reproduction procedure and hash verification. The
[Apache-2.0 license](LICENSE) and [NOTICE](NOTICE) cover this source; dependency
license/advisory acceptance remains a release gate. Full notices and their provenance
are in [dependency notices](docs/dependencies.md).

The [history review](specs/001-open-source-release/history-review.md) identifies
historical private-path/operational notes. Current-tree cleanup and secret scans
do not rewrite Git history. No push, tag, visibility change or publication is
part of the current feature implementation. Live preview signing/submission needs
separate authorization and must use disposable testnet credentials.
