# Minos Constitution

## Core principles

### I. Preserve funds, witnesses, and secrets
Transaction transformations must preserve original signed body bytes and existing
witnesses unless the operation explicitly changes them. Unsupported encodings must
fail before output is written. Preserve every asset through balancing. Secret
material must not leak into logs, reports, tests, or source history.

### II. Derive state from evidence
Metadata claims alone do not establish contract execution or authority. Indexing
must validate event provenance, isolate instances, handle rollbacks, and converge
to the same state after replay or restart. Unknown state must not look confirmed.

### III. Reproducible development and release
Use the pinned Nix environment and lockfiles. A clean checkout must include all
sources, fixtures, and checks needed to reproduce release artifacts. Do not scan
`/nix/store` or install development dependencies globally.

### IV. Test consequential behavior
Cover wallet/signing compatibility, value conservation, ledger-facing transaction
behavior, authorization, indexing lifecycle, and failure recovery. Prefer public
or synthetic fixtures and deterministic offline tests. Live testnet activity is
separate and explicitly authorized; tests must never submit to mainnet.

### V. Explicit boundaries and honest failures
Keep CLI parsing separate from workflow policy and transport. Share encoders,
validation, and query behavior where semantics match. Propagate errors affecting
correctness. Document supported commands/backends/networks and migration behavior;
never equate local preflight with full ledger acceptance.

## Workflow and scope

Use Spec Kit specification, plan, and tasks before feature implementation. Existing
completed artifacts remain valid unless superseded explicitly. Respect user stage
boundaries. No automatic signing, publishing, repository-visibility changes, pushes,
or subagents are authorized by this constitution. Follow the user's instruction to
explain a failure and ask before implementing a different approach.

## Governance

Current user instructions override these project conventions. Amendments must
record their reason and assess affected specs, tests, and compatibility. This
initial constitution records the working agreements and release review, rather
than introducing a separate approval process.

Version: 1.0.0 | Established: 2026-10-12 | Last amended: 2026-10-12
