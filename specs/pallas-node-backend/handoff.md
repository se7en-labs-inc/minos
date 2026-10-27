# Restart handoff — withdrawal validation and treasury script investigation

## Resolved after restart, 2026-10-12

The historical handoff below describes the earlier pause. The user subsequently
authorized removal of the surplus treasury reference and dependency downloads.
The treasury-contracts checkout at 84e105a9ff797fdf9873e0a5b71df9f108d8c008 was
inspected: vendor withdrawals use the registry, vendor and allowlist scripts;
the treasury reward-withdrawal handler explicitly rejects vendor inputs.
All three deployed fixture scripts match the compiled blueprint functions exactly
after one configuration argument. No evaluator checks were disabled.

Withdrawal construction, discovery and fees now exclude the treasury reference.
The historical fixture passes local evaluation with that reference removed;
redirecting its payout to the fee payer is rejected. All 28 workspace tests pass.
The 8,973 removed script bytes save 134,595 lovelace at the fixture's reference fee
rate, plus the smaller body fee. Original transaction files remain unchanged;
no signing or submission occurred. Rebuild a withdrawal to use the new behavior.

## Historical pause

Saved 2026-10-12 at the user's request. Read this before resuming.

## Current user intent

The maintainer asked to clean up Claude's code (replace cardano-cli subprocesses with Pallas),
review and conventionally commit the changes, then implement all review findings.
Five findings have been fixed and committed. Additional withdrawal preflight work
is implemented but uncommitted and requires contract investigation before finishing.

The latest exchange is important:

1. I said UPLC's optional preliminary check rejects an unused treasury reference.
2. The maintainer asked what I meant by the treasury script not executing.
3. I explained that the candidate transaction has no action invoking that hash,
   but acknowledged I had not proved this was correct for the contract.
4. The maintainer asked: **“so what is the purpose of the treasury script in this?”**
5. Investigation started but was interrupted by: **“why don't you write context
   and I'll restart you with access to the treasury contracts repo”**.

Answer the actual contract-purpose question using source and the deployed script
version before deciding whether to change the candidate transaction or evaluator.
Do not assume the treasury reference is unnecessary or that a treasury withdrawal
is missing. Neither conclusion has been established.

## Workspace and working agreements

- Repo: Minos checkout, branch `master`.
- User will restart with access to the treasury-contracts repository.
- Do not read wallet secrets, sign, or submit transactions. None were submitted.
- Use Nix/project dev shell. Do not enumerate `/nix/store`.
- No Python automation; prefer Rust.
- User requires asking before changing approach after failure. An async question
  about replacing UPLC's optional phase-one check was asked but **not approved**.
  Subsequent questions challenged our understanding; they are not approval.
- No subagents unless explicitly requested.
- Agentic-nix skill applies. Hybrid search/indexing failed with Ollama errors;
  user previously authorized direct local source fallback. BM25 works.
- Existing Spec Kit artifacts: `specs/pallas-node-backend/{spec,plan,tasks}.md`.
  Follow-up work extends these; no new branch/spec number was allocated.

## Commits

Initial cleanup/checkpoint commits from earlier work:

- `9cfaae2 chore(git): ignore local wallets and runtime artifacts`
- `abd288b feat(core): add treasury workflows and native Pallas node access`
- `8779a95 feat(maf): generate milestone acceptance forms from GitHub and reports`
- `88040bc feat(ui): add Elm treasury dashboard and Nix package`
- `3d14848 docs: document Pallas backend, workflows, and known limitations`

Review fixes committed this session:

- `1001807 fix(portal): authenticate and atomically register transactions without submission`
  - Verify an allowed vkey witness over original body CBOR hash.
  - Verify auxiliary-data hash so metadata is bound to the signed body.
  - Atomic Sled registration/instance tracking, idempotent by body hash.
  - Explicit registration-only `registerTransaction`; bundled `newTransaction`
    is an alias. CLI uses the explicit endpoint, never falls back to a remote
    newTransaction (which could also submit).
  - Registration failures produce CLI failure with `--portal-only` retry guidance.
  - Auth and idempotence tests, including configured inaccessible node proving
    registration does not attempt submission.
- `68c0c45 fix(node): share bounded Pallas sessions and preserve submission outcomes`
  - Shared library NodeSession; portal is now an adapter, not another backend.
  - Operation deadlines (30s default), bounded cleanup (1s), invalidation on errors.
  - Captured Tokio runtime handle allows Drop cleanup outside runtime thread.
  - Typed submission rejection vs unknown outcome with transaction hash; no retries.
  - CLI `--node-timeout-secs`, portal `node_timeout_secs`; reject magic/network conflict.
  - Preserve output CBOR for evaluator, protocol version/execution limits.
  - Koios HTTP deadline/status checks, latest epoch ordering, strict model parsing.
- `5f2d2e0 fix(tx): require complete reference inputs and verify script hashes`
  - `minos/src/references.rs`: batch/deduplicate refs, reject missing/extra/duplicate
    responses, require instance REGISTRY NFT, decode registry datum, verify vendor,
    treasury and staking script hashes, strictly count script bytes.
  - Applied to complete/withdraw/adjudicate/modify fee lookups; removed swallowed errors.
  - CLI now imports shared library modules, avoiding duplicate library/bin tests.
  - Public UTxO fixture committed; local HTTP mock covers incomplete/error/wrong-hash data.

No commits pushed.

## Uncommitted work — preserve it

- `minos/src/preflight.rs` (new)
- `minos/tests/withdrawal.rs` (new)
- `minos/tests/fixtures/withdrawal/{original.json,params.json,README.md}` (new)
- `minos/src/lib.rs`, `minos/src/main.rs`: expose preflight, CLI `preflight --tx`,
  automatic withdrawal preflight before writing unsigned output.
- `minos/src/koios.rs`: formatting-only residue after partial commit (inspect diff).
- README, portal example config, spec/plan/tasks updates.

Unrelated pre-existing untracked reports/documents and milestones JSON must remain
untouched: MAF files, daedalus-*.md/pdf, milestones*.json. Wallet/transaction files
are ignored and were not committed. Original user transaction file was not rewritten.

## Preflight implementation and its current limitation

`preflight::withdrawal` resolves spending/collateral inputs in one batch, reuses
already fetched reference UTxOs, and runs evaluation in spawn_blocking.
`preflight::evaluate` is a pure offline entry point. It:

- Decodes a Conway transaction and rejects unsupported transaction purposes.
- Checks complete resolved inputs, validity interval shape and value conservation.
- Reconstructs Koios output CBOR, preserving inline datum bytes and script bytes;
  node queries retain output CBOR directly.
- Supports V3 withdrawal transactions, protocol major 9..=11; rejects unknown versions.
- Checks declared aggregate budgets against protocol limits.
- Calls `uplc::tx::eval_phase_two_raw_with_protocol`, currently with
  `run_phase_one = true`, then checks unit return and per-redeemer budgets.
- Catches evaluator panics and fails closed. Does not sign, submit, alter budgets,
  or claim complete ledger validation (fees/min-UTxO/collateral/current TTL etc).

Pinned `uplc = 1.1.24` uses Pallas 0.35.1 internally; minos uses Pallas 1.4.0.
CBOR bridges those type versions. Do not upgrade dependencies casually.

Dependency source: `uplc` version 1.1.24. Resolve its local manifest with
`cargo metadata --format-version 1 --locked` inside the project Nix shell, or use
agentic-nix when source is unavailable locally. Registry cache paths are specific
to each developer's environment.

Key files:

- `src/tx.rs`: raw evaluation API and explicit protocol parameter.
- `src/tx/phase_one.rs`: `validate_missing_scripts` compares ALL lookup-table
  script hashes against scripts-needed, rejecting unused reference scripts.
- `src/tx/script_context.rs`: input/reward sorting and context generation.
  `sort_reward_accounts` orders by network, then **script before key**, then hash;
  do not substitute raw reward-address byte order if implementing local checks.
- `src/machine/runtime.rs`: protocol 11 selects V3 semantics E; 9/10 select C.

The real fixture fails BEFORE phase-two with:

    missing and/or unexpected validator(s) and/or redeemer(s)
          Missing ø
       Unexpected eb06997a94b339ee0b0dd0de7bfec2a184d1af577586654d44e90558

A possible solution proposed (not approved): locally verify exact required
scripts/redeemers, allow additional reference scripts, then call UPLC phase-two
with its optional preliminary check disabled. But FIRST determine the treasury
script's intended role from matching contract source. Do not weaken checks merely
to obtain a green result.

## Historical public fixtures

The offline regression data lives in `minos/tests/fixtures/withdrawal/`.
See its README for capture date, protocol version, contract source revision and
regression expectations. It includes a separate registry reference and allowlist
staking withdrawal. The original candidate is unsigned; it does not establish
ledger acceptance or current UTxO availability. Wallet associations and live
transaction handoffs are intentionally kept outside this development document.

## Treasury-contracts investigation — just discovered before interruption

Agentic-nix `list_repos` listed the `treasury-contracts` repository. It was not
available as a normal checkout in that session; the user planned to restart to
provide access. Resolve its current path from the available checkout or repository
listing. Index freshness and deployed version were unknown.

BM25 worked with the listed repository path and query `withdraw`, limit 5. It returned:

- `offchain/src/treasury/withdraw/` (exported by `offchain/src/treasury/index.ts`)
- `offchain/tests/allowlist/withdraw.test.ts`, especially lines 330-540:
  successful withdrawal examples call `withdraw({ configsOrScripts, blaze, now,
  inputs, destination: allowedAddresses[0], signers: [vendorSigner],
  additionalScripts: [{ script: allowlist.script.Script, redeemer: Data.Void() }] })`.
  Includes successful native-token-only withdrawal, matured payouts and metadata-only
  spend. Wrong-destination test expects `equal_plus_min_ada(vendor_outflow, target_inflow)`
  failure. This is a lead, NOT yet proof of how `withdraw()` itself invokes treasury.

BM25 query `allowlist_vendor` returned no results; not proof source is absent.
Next inspect the withdraw builder, vendor validator Withdraw branch, allowlist
validator and shared helpers, treasury validator branches, and matching tests.
Determine why offchain code adds treasury references and whether they serve a
reference-only inspection purpose or require treasury execution. Match deployed
script bytes/hashes or identify version uncertainty explicitly.

## Validation completed

Through `nix develop --command ...`:

- `cargo test --workspace --locked --offline`: **25 tests pass**
  (18 minos library, 5 withdrawal integration, 2 portal; bin now has zero duplicates).
- `cargo build --workspace --locked --offline`: passes.
- `cargo fmt --all`: applied; `git diff --check`: clean.
- No live node protocol test; mock Unix-socket tests only.
- No mainnet submission or signing.
- Existing unrelated compiler warnings remain; full Nix flake/clippy checks not run
  during this follow-up. Earlier UI Nix build passed.

The five withdrawal tests currently include an explicit regression test asserting
UPLC's surplus-reference rejection, NOT claiming the real transaction evaluates.
Synthetic V3 tests prove positive unsigned evaluation plus failure/budget checks.

Logs in /tmp may survive: minos-review-tests.log, minos-review-build.log,
minos-fixture-tests.log. Prefer committed fixtures/source over transient logs.

## Resume checklist

1. Read repository instructions and this handoff; preserve working tree and user files.
2. Inspect matching treasury-contracts source; answer The maintainer's script-purpose question.
3. Resolve the phase-one-check decision with that evidence and the user's agreement.
4. Finish meaningful positive/negative real-withdrawal preflight tests; do not claim
   the original mainnet failure fixed based merely on signatures, build, or mocks.
5. Update docs/spec tasks to final behavior, validate via Nix, commit remaining work
   conventionally. No signing/submission without new explicit authorization.
