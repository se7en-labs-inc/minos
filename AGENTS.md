# Minos development

Use the installed `speckit` skill for new features and specification/plan/tasks work.
Read `.specify/memory/constitution.md` and the active feature's `spec.md`, `plan.md`,
and `tasks.md` before implementation. The release-readiness feature is
`specs/001-open-source-release/`; implementation was authorized on 2026-10-26.

Use the project Nix shell. Primary checks:

- `nix develop --command cargo test --workspace --locked`
- `nix develop --command cargo fmt --all -- --check`
- `nix develop --command cargo clippy --workspace --all-targets --locked -- --deny warnings`
- `nix build .#minos .#minos-ui` and `nix flake check`

Use agentic-nix for code discovery. Preserve user wallet/transaction/report files.
Never read real wallet secrets or sign/submit transactions as a validation side effect.
Follow the user's global working agreements, including failure recovery, Nix,
worktree layout, and permission before changing approach after failure.
