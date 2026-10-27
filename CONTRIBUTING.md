# Contributing

Use the pinned Nix development environment. Do not install global Rust, Elm or
Python dependencies to work around build failures.

```sh
nix develop
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- --deny warnings
cargo fmt --all --check
nix flake check --no-update-lock-file
nix build --no-update-lock-file .#minos .#minos-ui --no-link
```

Run `nix fmt` to apply the repository's Rust, Elm, TOML and Nix formatting.
Commit intentional lockfile updates with their dependency changes. Package source
filesets must include any new test fixtures or test directories.

For features, update the applicable Spec Kit specification, design and tasks
before implementation. Keep CLI parsing, workflow policy, transport and ledger
validation independently testable. Describe the concrete behavior change and
relevant validation in a pull request.

Changes affecting funds, secrets, signing, transaction provenance or persistent
state need consequential regressions. Use published or synthetic fixtures with
clear attribution. Tests must run offline, never use a funded wallet and never
submit transactions. Live preview exercises require separate explicit approval.

Preserve signed transaction body bytes and existing witnesses. Reject unsupported
encodings before writing an artifact. Preserve every asset, use checked arithmetic,
and distinguish script evaluation from full ledger acceptance. Unknown indexer
state must remain visible rather than be presented as confirmed or withdrawn.

Do not include local wallet material, operational transaction files, databases,
personal filesystem paths or private reports. Report security issues privately as
described in [SECURITY.md](SECURITY.md).
