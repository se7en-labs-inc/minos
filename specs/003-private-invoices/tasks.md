# Tasks: Private invoices

- [x] T001 [US1] Define typed upload/session package, validation, deterministic payload and private file helpers in minos/src/invoice.rs.
- [x] T002 [US1] Implement offline review/sign/verify using existing COSE and wallet integration; test golden payload and tamper/key rejection.
- [x] T003 [US1] Implement private ticket/multipart/confirmation with persistent staged receipt and protocol mocks.
- [x] T004 [US2] Add explicit confirmation retry from receipt, without creating another ticket/upload; test ambiguous failure recovery.
- [x] T005 [US3] Implement scoped session unlock, authenticated list and hash-checked private download; test session scope/expiry and wrong bytes.
- [x] T006 Integrate invoice CLI with pre-network offline dispatch and generated completions; test real CLI offline paths with synthetic keys.
- [x] T007 Document commands, required claims, privacy, two-minute freshness and recovery in docs/invoices.md; link README.
- [x] T008 Run required Nix workspace/package/flake checks, inspect final diff, and record results. No live signing/upload/submission.

## Validation — 2026-11-24

- `nix develop --command cargo test --workspace --locked`: 164 tests passed.
- `nix develop --command cargo fmt --all -- --check`: passed.
- `nix develop --command cargo clippy --workspace --all-targets --locked -- --deny warnings`: passed.
- `nix build .#minos .#minos-ui --no-link --print-out-paths`: passed; release package also ran all 164 Rust tests successfully.
- `nix flake check`: passed, including packaged Clippy, formatting and UI checks.
- Installed `minos invoice prepare-package --help`, invoice runbook and Bash completion readback: passed.
- `git diff --check`: passed. Final source review covers payload binding,
  private output permissions, staged receipts, bounded transfers, error redaction,
  session scope and unchanged public evidence/transaction behavior.

Built CLI: `/nix/store/0aplkqyqjjafcp9916f78f3cspvlh873-minos-0.1.0/bin/minos`.
No live invoice was uploaded during the automated checks and no production wallet
was read.

## Live preview acceptance — 2026-11-25

- User authorized the existing local preview wallet, account 0.
- Synthetic 50 test ADA invoice privately uploaded and confirmed on the existing
  Se7en preview project CTW-26-0024, milestone m-8.
- Separate signed unlock, authenticated listing and download passed.
- Downloaded PDF SHA-256 matched the original.
- No Cardano transaction, withdrawal or mainnet write. USDM unit acceptance
  remains untested.
