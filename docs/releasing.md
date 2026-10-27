# Release procedure

Release candidates must satisfy the existing feature acceptance checklist. A
successful Rust build alone is insufficient. Keep failures and accepted support
limits visible; do not disable gates to create a release.

1. Finish `specs/001-open-source-release/tasks.md` and review the support matrix,
   operations/migration instructions, changelog and embedded-contract provenance.
2. In a clean checkout, run `nix develop --command cargo test --workspace --locked`,
   `nix develop --command cargo clippy --workspace --all-targets --locked -- --deny warnings`,
   `nix fmt -- --fail-on-change`, and `nix flake check -L`. Build the packaged CLI,
   portal and UI; run integration tests outside the development shell and record
   output identifiers and runtime dependency evidence in the release checklist.
3. Repeat the history and exported-artifact secret/privacy scans. Preserve approved
   author name/email. If history contains unintended private material, prepare and
   review a concrete sanitized history/export before any separately authorized
   rewrite. Scanning the current tree does not clean Git history.
4. Audit the locked dependency graph and license files, including bundled native
   code, GPG/runtime dependencies, Elm packages and reproduced contract sources.
   Fix actionable advisories; record reachability/target analysis and explicit
   maintainer decisions for exclusions. Generate and ship third-party notices.
5. Rehearse backup, restore, migration and reindex with local registrations retained.
   Prepare the preview lifecycle using disposable wallets. Obtain separate approval
   before signing or submitting preview transactions, record public transaction
   evidence, and verify CLI → node → portal → UI behavior and rollback recovery.
   The concrete sequence and public evidence fields are in
   [preview-rehearsal.md](preview-rehearsal.md).
6. Have the maintainer review all acceptance evidence, version/support decisions,
   dependency findings and remaining exclusions. Only after all mandatory gates
   pass should a separate publication request authorize tagging/pushing a release
   or changing repository visibility.

No release, tag, history rewrite, network submission or visibility change is
performed merely by completing this checklist or building a package.
