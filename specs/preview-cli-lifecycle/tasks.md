# Tasks

- [x] P017 Add/test evidence labels and filename defaults; resubmit September MAF
  with actual uploaded CID, preserve all funds, verify portal label.

- [x] P016 Correct withdrawal author metadata and completion reference override;
  validate, withdraw Base M0/M1 and upload/submit requested preview MAF.

- [x] P014 Add validated threshold publishing with matching metadata, separate
  vendor and funding co-signers; test and build using Nix.
- [ ] P015 Derive accounts0/1/2 admins, account3 vendor; publish/initialize
  Example Treasury Multisig, sign funding with accounts0/1/3, submit through portal
  and verify its pending/on-chain status without changing routes silently.

- [x] P013 Create Example Treasury Base using base-address inputs/collateral/change,
  initialize its 950 ADA treasury and submit the ADA project through the portal.

- [x] P010 Add/test fixed-ADA cloning and ADA-only treasury funding selection.
- [ ] P012 Match confirmed initialize metadata; annotate existing 950 ADA via
  treasury Reorganize, submit through portal and retry project allocation.
- [ ] P011 Deposit 950 ADA and submit the 19x50 ADA project through the portal;
  verify response and indexing without a direct project-broadcast fallback.

- [x] P007 Add/test existing-asset minting; top up USDM7, leave USDM untouched.
- [x] P008 Match portal funding metadata list/identifiers and preserve M0=false.
- [x] P009 Add treasury deposit and correct treasury-spend project funding;
  validate value conservation and real script execution before submission.

- [ ] P001 Finish integrated Minos airgap CLI cleanup/tests and update active context.
- [ ] P002 Add minimal portal query/snapshot and status functionality; recognize
  remote String Ok acknowledgments. Verify incoming separate collateral.
- [x] P003 Publish and confirm/index Example Treasury through the preview portal.
- [x] P004 Clone and fund mainnet project details with immediate M0 and later
  15-minute unlocks; confirm project indexing and amounts.
- [x] P005 Withdraw M0 without MAF, then sign/upload/submit applicable MAF evidence.
- [ ] P006 Confirm evidence, a later withdrawal, and resulting project/wallet state.

Operational transaction identifiers and wallet associations are retained outside
the source tree. These historical task statuses do not authorize new live work.
