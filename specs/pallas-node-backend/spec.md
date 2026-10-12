# Pallas node backend cleanup

Status: implementation authorized, 2026-10-11

This refactors the existing uncommitted CLI backend in the current worktree; it
does not allocate a new numbered feature or move the user's pending work.

## User stories and acceptance

1. P1: Build transactions with a node socket and no cardano-cli installed.
   Given a supported node, tip, protocol parameters, address UTxOs and explicit
   output references are queried through Pallas node-to-client protocols.
   Inline datums, native assets and reference-script bytes survive conversion.
2. P1: Submit signed Conway transactions through the selected node backend.
   Acceptance returns the body hash; rejection retains the node's error.
   Signing and submission remain explicit commands, never validation side effects.
3. P2: Failed requests release connections. Invalid addresses/references and
   missing protocol fields fail explicitly. No hardcoded binary path or subprocess
   discovery remains. Koios remains the backend when no socket is configured.

## Constraints and edge cases

- Preserve current withdrawal changes and all wallet/transaction artifacts.
- Asset-by-policy discovery is not a filtered local-state query; report this
  limitation instead of scanning the complete ledger or silently using Koios.
- Preserve the chain slot's actual time for mainnet, preview and preprod; reject
  unknown network time mappings and an origin tip rather than inventing a time.
- Require Conway for protocol parameters and transaction submission.
- Exercise protocol integration with a local mock node; a live socket is not
  available in this environment. Never submit a mainnet transaction in tests.

## Success criteria

- No cardano-cli process invocation or binary option remains in executable code.
- Workspace tests and targeted node-protocol tests pass through Nix.
- Documentation describes direct Pallas operation and its remaining limitations.

## Authorized review follow-up (2026-10-12)

- P1: Reference-script fee queries fail closed on errors, missing outputs, malformed
  scripts and unexpected script hashes; repeated references count once.
- P1: Portal registration verifies an allowed author's Ed25519 witness over the
  original body bytes before any write, including when no node is configured.
- P1: CLI and portal share guarded Pallas sessions. Failure, cancellation and
  timeout release the multiplexer; operations and cleanup have bounded deadlines.
- P2: Registration never submits; repeated registration by body hash is idempotent.
  Submission failures and registration failures remain distinct to the caller.
- P1: Withdrawal generation evaluates scripts locally before writing the unsigned
  artifact, resolving inputs/reference inputs in a batched read. Missing data,
  value imbalance, failed scripts and insufficient execution budgets stop output.
  No signing, submission or automatic retry occurs during preflight.
- Regression fixtures cover withdrawal datum changes, reference and staking scripts,
  value conservation, signature spoofing, timeouts and protocol cleanup. Public
  transaction data may be fixtures; wallet files and secrets must never be read.

Vendor withdrawals must omit the unused treasury script reference and exclude its
bytes from fee calculation, retaining the registry, vendor and required staking
references. This removal was explicitly authorized on 2026-10-12.
