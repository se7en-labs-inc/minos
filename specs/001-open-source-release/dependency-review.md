# Locked dependency review — scoped maintainer acceptance

Scope: Cargo.lock, x86_64-linux package graph, pinned Nix build and Elm package
lock. The maintainer accepted candidate 0.1.0's documented Linux/input exclusions
on 2026-10-27. This does not claim an advisory-free lockfile.
No blanket RustSec ignore list has been introduced.

## RustSec remediation

The initial audit used database revision
`a78282a1414d6c28cc228c4eae3570451affb0dc` (2026-10-26), covering 458 locked
packages. It reported 13 vulnerability entries plus informational warnings.
The user approved targeted updates; post-remediation audit reports seven existing
transitive entries and unmaintained warnings. TLS/Tokio findings and the newly
reported yanked dependency warning are gone.

| Dependency | Finding | Implemented disposition or remaining decision |
|---|---|---|
| rustls 0.23.45 → 0.23.46 | RUSTSEC-2026-0346, 0347, 0348, 0349 | Patched; no longer reported |
| rustls-webpki 0.103.15 → 0.103.16 | RUSTSEC-2026-0382 | Patched; no longer reported |
| webpki-roots 1.0.9 → 1.0.10 | RUSTSEC-2026-0381 | Patched; no longer reported |
| tokio 1.53.1 → 1.53.3 | RUSTSEC-2026-0386 unsoundness warning | Patched; no longer reported |
| yoke-derive 0.8.3 → 0.8.4 | Yanked macro dependency | Patched; no longer reported |
| crypto-bigint 0.5.5 | RUSTSEC-2026-0387 | Wasm-specific uplc/k256 path, absent from the filtered Linux graph; retain Linux-only target scope and review upstream remediation |
| zip 6.0.0 and zip 8.6.0 | RUSTSEC-2026-0367, 0368, 0369 for each | zip 6 is associated with DuckDB's disabled download/extraction path in the bundled build; zip 8 is used by the DOCX writer and trusted generated-document tests. No application untrusted archive-extraction path was identified; maintainer acceptance remains required |
| fxhash, instant, paste | Unmaintained dependency warnings | Transitive owners and upstream replacement policy remain part of maintainer review |

The remaining fixes require upstream major-version changes (ZIP ≥9 and
crypto-bigint ≥0.7.6), beyond the approved compatible patch updates. Target and
call-path analysis does not remove an advisory from Cargo.lock or authorize a
new platform/input surface. No RustSec suppression was added; the unfiltered
cargo-audit command still exits nonzero. Do not call this a clean zero-advisory
audit. The accepted exclusions apply to x86_64-linux without untrusted archive
extraction. New targets or extraction features require a fresh dependency review.
Final integrated live-preview release acceptance remains deferred.

## License inventory and notices

The locked online cargo-about 0.9.0 audit completed successfully for the Linux
workspace with `--locked --fail`: 383 crates and 111 license texts. The earlier
offline failure requested an uncached Android dependency despite target filtering;
that failure is superseded by the successful authorized online run.

Full license texts/copyright notices and versioned crate source links are in
`THIRD_PARTY_NOTICES.md`. Report configuration/template are versioned. No local
package/cache paths are included in the public report. `ascii_utils` 0.9.3 and
`fast_chemail` 0.9.6 retain MPL-2.0 and direct source links; dependency code was not
modified. The accepted license expressions are report selections, not automatic
maintainer legal acceptance.

The collection also includes all10 locked Elm package licenses, DuckDB 1.5.5's
main and 28 third-party upstream license files, GNU GPL/LGPL/runtime-exception
texts and pinned native component metadata. `licenses/SHA256SUMS` verifies copied
bytes. Original upstream whitespace/line endings are retained in imported notices;
code formatting checks do not rewrite those files. See `docs/dependencies.md`
for provenance, regeneration and binary-redistribution boundaries. Verify notice
installation in the clean package and keep final maintainer decisions explicit.
