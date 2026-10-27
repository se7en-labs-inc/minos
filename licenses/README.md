# License collection provenance

- `elm-NAME-VERSION.txt`: exact `LICENSE` from
  `https://raw.githubusercontent.com/elm/NAME/VERSION/LICENSE` for the ten packages
  in `minos-ui/elm.json` and `elm-packages.nix`.
- `duckdb-1.5.5.txt`: `https://raw.githubusercontent.com/duckdb/duckdb/v1.5.5/LICENSE`.
- `duckdb-1.5.5/SOURCES`: paths obtained from the complete GitHub tree at
  `https://api.github.com/repos/duckdb/duckdb/git/trees/v1.5.5?recursive=1`, rejecting
  truncated results. Each listed file is copied unchanged from that tag under its
  original relative path.
- `GPL-3.0.txt`, `LGPL-3.0.txt`: GNU's `https://www.gnu.org/licenses/` text files.
- `LGPL-2.1.txt`: `https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt`.
- `GCC-exception-3.1.html`: `https://www.gnu.org/licenses/gcc-exception-3.1.html`.
- `openssl-3.6.4.txt`: exact `LICENSE.txt` from the upstream `openssl-3.6.4` tag.
- `zlib-1.3.2.txt`: exact `LICENSE` from `madler/zlib` tag `v1.3.2`.
- `native-runtime.json`: name/version/license metadata evaluated from the locked
  Nixpkgs input; no local source paths are retained.
- `about.hbs`: cargo-about template generating the root Rust notices without local
  package paths. It preserves the complete license texts and associated crate versions.

`SHA256SUMS` records downloaded file bytes. Copyright and upstream attribution are
intentionally preserved. These files are notices and source references, not an
assertion that every optional component is present in each binary.
