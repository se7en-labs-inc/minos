# Plan

Add complete --evidence-label in matching order with --evidence-url, defaultempty
for existing callers, autofilename for --evidence-file. Pass labels through
CompleteParams and encode_complete, UTF8chunk as othermetadata. Validate count
and test decoded label/URL associations and longfilenames. Rebuild/sign/register
and broadcast using existingCID, no duplicate upload. Inspect exact validity
requirements if a matured active payout blocks zero-withdraw completion; no
extra withdrawal without authorization.

Verified against the actual vendor validator: zero-withdraw completion can span
an active payout maturation, provided validity lower bound precedes that payout.
Use min(tip-120, earliestActiveMaturationSlot-1), ordinary2hupperbound and local
preflight. This matches UI's earlierlowerbound behavior, preserves allpayouts and
removes the unnecessarily restrictive wallclockmatured guard/upperboundcap.

Fix withdrawal txAuthor using first vendor payment key (already required signer),
validate before writing and test decoded metadata. Extend complete --publish-tx
to resolve missing vendor reference consistently with withdraw, retaining hash
verification/local preflight. Sequential withdrawal confirmation before building
completion prevents spending stale vendor UTxO. Upload via unified package.

Extend publish with repeated --admin-key and --admin-threshold, default1 for
compatibility. Derive one validated permission tree shared by script parameters
and publish metadata. Fund-project adds --vendor-key and repeated --co-signer;
preserve txAuthor and select signing admins explicitly, with fee-owner signer.
Existing wallet sign appends witnesses sequentially from accounts0/1/3.
Test threshold metadata/script consistency, malformed settings, distinct vendor
datum and signer list. Derive public accounts via Minos, bootstrap new budget
through established Koios path, initialize950ADA and submit funding via portal.

After ADA-only funding and correctly annotated wallet initialization still fail,
prepare a same-payment-key base-address change diagnostic. Add wallet address
--base and fund/deposit --change-addr; verify payment credential/network match.
Keep the enterprise wallet as the input query address. Obtain approval before
submitting this new address-format experiment under the failure agreement.
User approved investigating base versus enterprise; base-change-only attempt
still fails. Complete the same experiment using base-address fee/collateral
inputs. Add wallet send to build an ADA-only transfer with multiple equal
outputs, preserve assets by selecting only ADA-only inputs, then fund two 10
ADA outputs at this account's base address. Project submission stays through
the portal. No new budget or large deposit for this diagnostic.

Compare confirmed Marshall treasury deposits with our deposits by fetching their
public transaction CBOR through `minos wallet fetch` over Koios. Verify returned
CBOR against the requested body hash before writing a new local JSON envelope.
Confirmed Marshall deposit metadata: event initialize, outputs {"0":{identifier}}.
Add it to deposit-project. Support --reuse-treasury HASH#INDEX for an exact
ADA-only existing deposit: Reorganize that UTxO to the same treasury/value with
initialization metadata, separate wallet fees/collateral, local evaluation and
portal submission. This avoids transferring another 950 ADA from the wallet.

Current ADA attempt: add clone-project --ada-per-milestone and --project-id,
preserving names/criteria while replacing all payout values with fixed ADA.
Select one ADA-only treasury UTxO and separate ADA-only wallet fee/collateral
inputs; never pull other treasury outputs into wallet fee selection. Deposit
950 ADA, refresh the schedule, locally evaluate, sign and submit via portal.

Add `mint --existing-asset` to issue only fungible units under the same native
policy; preserve the existing reference NFT. Test the decoded mint map for both
initial issuance and additional issuance before signing preview transactions.
Funding metadata uses the public client's ordered milestone list, explicit
identifiers and integer `evidenceOptional: 1` for M0. This preserves the source
evidence settings during project creation without a separate mutation.

Correct project funding to spend a budget treasury UTxO with Fund(amount),
registry and treasury reference inputs, separate wallet fees/collateral and
locally evaluated execution. Add deposit-project to transfer the template's
test tokens to the treasury before funding. Return unused treasury value to
the treasury and preserve exact payout amounts. Project submission remains
through the portal; inspect public CBOR using wallet inspect for diagnostics.
Inspect preview's public UI bundle through a Minos portal-client command when
the GraphQL schema omits supported-currency configuration. Preserve the user's
USDM7 choice until any currency change is explicitly agreed.

Continue in current dirty worktree, preserve unrelated files. Reuse publish,
fund, complete, withdraw, wallet sign, integrated minos airgap and upload-package.
Finish existing integration cleanup first. Add minimal Minos portal-query/status
snapshot and transaction-status commands for black-box schema/status checks, and
fix the observed remote String "Ok" acknowledgment alongside existing Boolean
support. No separate executable or deployment archive.

Check incoming collateral using Minos chain query. Publish Example Treasury and
register through remote preview portal. Confirm actual inclusion/indexing before
funding. Use own preview wallet and HTTP chain provider where protocol data is
needed; no local node. Inspect mainnet project read-only using Minos for a faithful
clone. Extend CLI template/schedule support only as necessary to preserve full
project metadata and 15-minute timing. Prefer existing CLI commands otherwise.
Set timings only when ready to fund so M0 is immediate and later unlocks have a
full 15-minute interval. Reuse signed PDF for preview MAF upload. Sign the combined
package locally with explicitly authorized preview mnemonic (not mainnet wallet).
Keep tests focused on signature/balance/metadata and portal boundaries. Run checks
through Nix. Actual preview operations must be Minos CLI invocations only.
