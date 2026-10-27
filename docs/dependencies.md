# Dependency notices and review

The source is Apache-2.0. Third-party components retain their own licenses.
The locked Linux Rust graph was inspected with cargo-about 0.9.0: 383 crates and
111 associated license texts. Full texts, copyright notices and versioned source
links are in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md), including the
unmodified MPL-2.0 `ascii_utils` 0.9.3 and `fast_chemail` 0.9.6 sources. The inventory
conservatively includes build/test dependencies as well as linked components.

Regenerate with the pinned Nix environment after any Cargo.lock change:

```sh
nix develop --command nix shell --inputs-from . nixpkgs#cargo-about --command \
  cargo-about generate --config about.toml --workspace \
  --target x86_64-unknown-linux-gnu --locked --fail \
  --output-file THIRD_PARTY_NOTICES.md licenses/about.hbs
```

`about.toml` records the license selections for the report, not a blanket legal or
security acceptance. The original machine-readable report includes local package
paths and is kept outside release source. Do not commit it without sanitizing it.

## Native code and packaged tools

Bundled `libduckdb-sys` 1.10505.0 contains DuckDB 1.5.5, as specified by the
[upstream versioning scheme](https://docs.rs/crate/libduckdb-sys/1.10505.0).
Its Cargo source archive omits standalone license files. Therefore the collection
includes the [upstream DuckDB license](../licenses/duckdb-1.5.5.txt) and 28
upstream third-party license/notice files from the exact `v1.5.5` tag under
[licenses/duckdb-1.5.5](../licenses/duckdb-1.5.5/SOURCES). This conservative collection
also includes optional upstream components not enabled by this build. The JSON
extension is compiled in; no runtime extension download or database service is used.

The pinned Nixpkgs revision is `7fc6f2c20af09cdcaf48b92ec3121860139ec668`.
[Native runtime metadata](../licenses/native-runtime.json) records:

| Component | Pinned version | Nixpkgs license identifier |
|---|---|---|
| GnuPG | 2.4.9 | GPL-3.0-or-later |
| GCC/C++ runtime | 15.3.0 | GPL-3.0-or-later; review the supplied GCC runtime exception |
| OpenSSL build/check dependency | 3.6.4 | Apache-2.0 |
| zlib | 1.3.2 | Zlib |

GNU GPL/LGPL texts, the GCC runtime exception and OpenSSL/zlib license files are
included under `licenses/`. The GCC exception was obtained as the GNU HTML page
because that site's corresponding `.txt` URL does not exist. Runtime licensing is
not relabeled as Apache-2.0. The Nix derivations retain the pinned source recipes
for the full dependency closure. Before redistributing standalone binaries or a
copied runtime closure, review that exact closure's corresponding-source and notice
requirements; this source inventory does not certify a new distribution method.

## Elm application

The ten locked Elm packages are all from the upstream `elm` organization: browser
1.0.2, core 1.0.5, html 1.0.0, http 2.0.0, json 1.1.3, time 1.0.0, bytes 1.0.8,
file 1.0.5, url 1.0.0 and virtual-dom 1.0.3. Their complete upstream license texts
are included as `licenses/elm-NAME-VERSION.txt`. The UI package installs them
alongside its own LICENSE/NOTICE; it does not rely on the development checkout.

## Advisories and final acceptance

TLS and Tokio remediation is recorded in the
[dependency review](../specs/001-open-source-release/dependency-review.md).
Remaining target/feature-unreachable archive/Wasm findings and maintenance warnings
are explicit review items, not hidden ignores. Final license/support/advisory
acceptance belongs to the maintainer release gate. No dependency source was modified
as part of the notice collection.
