# Pinned contract blueprint

`blueprint.json` contains the preamble and the oneshot mint, treasury spend and
vendor spend validators extracted from the upstream `plutus.json` at
SundaeSwap-finance/treasury-contracts revision
`84e105a9ff797fdf9873e0a5b71df9f108d8c008`.

The unmodified upstream blueprint SHA-256 is
`27f12fffcef47027d6f590024797ec079c411c7fef26757e556f1db3724b55a6`.
Extraction preserves the exact compiledCode and declared hashes. The provenance
test compares embedded assets byte-for-byte and checks their Plutus V3 hashes.

The upstream source declares Apache-2.0. See the root NOTICE and
`docs/contracts.md` for compiler and dependency provenance. Historical applied
script fixtures are separate from these unparameterized base scripts.
