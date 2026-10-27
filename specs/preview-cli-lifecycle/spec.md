# Preview lifecycle using only Minos

Latest steering: test separate mnemonic accounts as three administrators and one
vendor. Create Example Treasury Multisig with accounts0/1/2 as 2-of-3 administrators
and account3 as vendor, all using preview base addresses. Register matching
threshold permissions in both on-chain scripts and discovery metadata. Preserve
existing instances. Fund the same 19x50ADA milestones with two admin signatures
and the vendor signature through the portal; report saved versus broadcast state.
Reject duplicate/malformed admin keys and invalid thresholds before any network
action. Distinct accounts from one seed test roles, not independent key custody.

Current steering pauses multisig: withdraw Base M0/M1 (100ADA), upload existing
mainnet PDF as preview MAF to user-confirmed milestone. Portal withdrawal metadata
must bind txAuthor to a required vendor signer. Completion must accept known
publish reference when portal scriptReference is absent, verifying actual hashes.

Resubmit September MAF with its actual filename as evidence label and the existing
uploaded CID. Complete supports per-URL evidence labels; local evidence files
automatically use their filename. Preserve all existing payouts and do not claim
additional matured milestones merely to change a label. Verify indexed label.

Create a fresh preview budget named Example Treasury through the remote Sundae
preview portal. Clone the mainnet project's identity, description and all
milestone labels/details/amounts, mapping test assets if needed. M0 is eligible
immediately on funding; subsequent milestones unlock every 15 minutes. Withdraw
M0 without a MAF, upload/submit the existing project MAF for its applicable later
milestone, then verify a later matured withdrawal. Use only Minos CLI operations
for portal/chain interaction; do not use a node socket or curl/cardano-cli helpers.
Live preview signing requires separate explicit authorization and disposable
credentials. Do not print mnemonics or use a mainnet signing wallet.

The first project was funded on chain with USDM7 but remains invisible in the
portal. User now requests a separate ADA-only attempt through the portal: 50 ADA
for each of the 19 milestones (950 ADA total), same project/milestone text and
evidence settings, M0 immediate and later +15 minutes. Use project identifier
EXAMPLE-001-ADA to avoid colliding with the existing on-chain project; preserve
the display name and description. Leave both existing USDM7 funding and the
mistaken USDM issuance untouched. Do not bypass portal submission for this
project without further authorization. An ordinary budget deposit may use the
established Minos/Koios deposit workflow.

The user is supplying a second ADA-only UTxO (>=5 ADA) for separate collateral.
Do not add a wallet-split feature. No new mainnet writes are authorized by this
preview test. Existing mainnet requests are not the active signing request.

Latest user steering: create a new budget named Example Treasury Base, using the
base-address form of the same preview wallet for fee inputs, collateral and
change. Prepare the ADA-only 19x50 project there. Leave the original budget and
its existing on-chain projects/deposits untouched. Publish and initialize using
Minos/Koios where required by the protocol; project allocation goes through the
portal. Do not change payment key or network.

Acceptance: actual budget/project visible in remote preview portal; M0 withdrawal
confirmed without MAF; real portal PDF upload and matching CID verified; evidence
indexed for the applicable milestone; later withdrawal proven with chain hash
and wallet/project value changes. Report portal acceptance separately from chain
inclusion. Explain failures and ask before implementing a different approach.
