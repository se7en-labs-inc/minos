# Withdrawal regression data

These are public chain data for CTW-26-0024, instance
`51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e`.
No wallet material or signing witnesses are included.

- `original.json`: snapshot of the user's unsigned
  `se7en-withdraw-initial.txbody`, captured 2026-10-12. It already includes the
  corrected separate registry reference and allowlist staking withdrawal.
  It is a historical candidate, not evidence of ledger acceptance.
- `utxos.json`: seven outputs resolved through Koios `utxo_info` with
  `_extended: true`, captured 2026-10-12 in three sequential batched queries.
  Includes vendor input, fee input, collateral, registry and three script refs.
- `params.json`: Koios `epoch_params?limit=1&order=epoch_no.desc`, epoch 661,
  protocol major 11; retains the protocol fields relevant to this test.

Tests run entirely offline. Fixture values are historical and must never be used
as live UTxO availability, current protocol parameters or submission approval.
The evaluator is pinned to uplc 1.1.24. Its explicit protocol API selects V3
semantics E at major 11 (runtime.rs), rather than applying its default semantics
implicitly to earlier protocol versions.

The historical candidate is preserved unchanged. A regression removes only its
treasury reference in memory and verifies successful vendor and allowlist
evaluation with UPLC's preliminary checks enabled. Sending that payout to the
fee payer instead is rejected. Builder and resolver tests verify that new vendor
withdrawals omit the treasury reference and its 8,973 bytes from fee accounting.

Source investigation used treasury-contracts commit
`84e105a9ff797fdf9873e0a5b71df9f108d8c008`. Parsing each deployed fixture script
as UPLC showed an application whose function exactly equals the corresponding
`plutus.json` validator and whose argument supplies its configuration (same UPLC
version). Matches: treasury `eb06997a…`, vendor `5bab145a…`, allowlist `326cec60…`.
The upstream vendor withdrawal builder attaches no treasury script reference.
