# Contract provenance

The embedded scripts come from
[SundaeSwap-finance/treasury-contracts](https://github.com/SundaeSwap-finance/treasury-contracts)
revision `84e105a9ff797fdf9873e0a5b71df9f108d8c008`.
`aiken.toml` selects compiler `v1.1.19`, Plutus V3 and the default build
configuration. The published blueprint records `v1.1.19+unknown`.

Locked Aiken dependencies are `aiken-lang/stdlib` v2.2.0,
`aiken-lang/fuzz` v2.1.1 and `SundaeSwap-finance/aicone` 504e104.
The source declares Apache-2.0 in both its manifest and blueprint; that revision
has no separate LICENSE or NOTICE file. See the root NOTICE for attribution.

| Upstream file | SHA-256 of original bytes |
|---|---|
| `aiken.toml` | `896c1c023a04e8ee3464f5be140686165b6da041982e4dcdd5b6623df0ced5d8` |
| `aiken.lock` | `c247c4a6bd0ed69c183b7df06915d948817ac9f8cc1f9604341049099d11bb81` |
| `plutus.json` | `27f12fffcef47027d6f590024797ec079c411c7fef26757e556f1db3724b55a6` |

`minos/src/contracts/*.hex` are newline-terminated compiledCode values extracted
without changing their bytes. The reduced independent blueprint fixture keeps
the upstream preamble, titles, compiledCode and hashes. Its extraction is:

```sh
jq '{preamble, validators: [.validators[] |
  select(.title == "oneshot.oneshot.mint" or
         .title == "treasury.treasury.spend" or
         .title == "vendor.vendor.spend") |
  {title, compiledCode, hash}]}' plutus.json
```

| Validator | Unparameterized Plutus V3 hash |
|---|---|
| `oneshot.oneshot.mint` | `f81cb409f646e4e94d09e8888a7196378cc6c5587d581ce7124f608a` |
| `treasury.treasury.spend` | `03d983a118dbba6bb2d54a5a7ce7757c8a646964e7ffb0193bf7f6ed` |
| `vendor.vendor.spend` | `b98f5e0fdad30510cc232e5f6777685b6cfce3b7268f197987fd9a6a` |

Run the offline provenance regression with:

```sh
nix develop -c cargo test --locked -p minos provenance_tests
```

This proves exact equality to the pinned published blueprint and its declared
hashes. Rebuilding from Aiken source was also verified as recorded below.

## Rebuild procedure

Use a disposable checkout of the pinned contract revision. Keep its original
blueprint separately for comparison. Build with Aiken source revision
`e525483dd81e10861911a4ec320f05b30e586324` (v1.1.19), whose upstream Nix flake
has NAR hash `sha256-S3KIOlOz21ItWI+RoeHPYROIlMbKAoNi7hXwHHjHaJs=`:

```sh
nix shell github:aiken-lang/aiken/e525483dd81e10861911a4ec320f05b30e586324 -c aiken build
```

Use the source's `aiken.lock` and default configuration; do not substitute a newer
compiler or edit validator parameters to obtain a matching hash. Compare the three
compiledCode fields and hashes, rather than the entire blueprint preamble, since
the compiler build records its actual source revision instead of `unknown`.

Verified on 2026-10-26: `aiken v1.1.19+e525483` rebuilt the pinned source
successfully with the original lockfile unchanged. All three selected validator
titles, compiledCode fields and hashes compare identically to the original
blueprint. Compiler output is
`/nix/store/rs9sa7ywqg7wh5yvc9v3sjhx0ys4b9nv-aiken`, with NAR hash
`sha256-OL1Tggmdm8D7IPEL/B6F9YRO77xuF6O0bWiBY+9dVT4=`.
The initial compiler and dependency downloads need network access; Minos's
embedded-script regression runs offline after checkout.

## Applied scripts

Base hashes are not deployment hashes. `scripts.rs` encodes the oneshot seed
outref, registry policy, permission trees and expirations as PlutusData, and
`uplc.rs` applies that parameter to the base program. The resulting CBOR-wrapped
program is hashed with the Plutus V3 language tag. Distinguish that representation
from its unwrapped flat bytes.

Historical withdrawal fixtures contain applied treasury, vendor and allowlist
scripts. Their fixture README records the source comparison and deployed linkage.
The real-script regressions check their actual behavior and expected destinations;
those public chain fixtures remain linkable and are not anonymized. Never treat
their historical output references as current availability or deployment defaults.
