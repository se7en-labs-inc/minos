# Private milestone invoices

`minos invoice` follows Sundae's private invoice API. It attaches an invoice to a
milestone without publishing evidence metadata or building a Cardano transaction.
It does not replace the signed MAF or withdraw a payment.

Use current Minos on the online machine and airgap. From a checkout, enter
`nix develop`, run `cargo build -p minos --locked`, and use
`./target/debug/minos` in place of `minos`. GPG wallet support is unchanged.

## Prepare the document and claims

Intersect's [invoice guide](https://admin-services.docs.intersectmbo.org/guides/vendor-guides/2-delivery-and-milestone-management/how-to-submit-an-invoice)
requires supplier name/address, billing customer/address from the agreement,
invoice number/date, project number, contractual milestone name/number, fixed
price/currency, and the wallet address for ADA payments. Its filename convention
is `Invoice - Supplier Name - Project Number - YYYYMMDD.pdf`.

Use the amount and currency in the signed agreement. The current dashboard and
API require invoice number, invoice date, due date and amount, even though the
written guide describes some form fields as optional. Minos requires those
claims explicitly. Dates use YYYY-MM-DD; the due date cannot precede the invoice
date; amounts are decimal strings with up to six places, never floating point.
Notes are optional (up to 1000 characters).

The current web client sends `unit: "lovelace"` for a decimal **ADA** amount:
1250.75 ADA is sent as `amount: "1250.75"`, not 1250750000. Minos preserves the
explicit amount and unit without conversion. The API exposes a string unit;
Minos does not infer token units from tickers, exchange currencies or assert that
another unit is accepted by the server. Confirm the portal unit identifier with
the administrator for a non-ADA agreement; do not label USDM as ADA.

PDF, PNG and JPEG files are supported, up to 10 MiB. Extension and file signature
must agree. Minos validates the attachment and claims, not the invoice's legal or
accounting content. Keep all operational files out of Git. New invoice files use
mode 0600 and receipt directories use 0700 on Unix. Existing paths and symlinks
are refused; preserve previous attempts under other names before reusing a name.

## Prepare online

Select the network explicitly (default is preview). Read `minos status` first to
check the project/milestone. Replace every uppercase placeholder below; the
amount/unit example is for an ADA agreement only.

```sh
minos --network mainnet invoice prepare-package \
  --instance INSTANCE_POLICY_ID --project PROJECT_ID --milestone MILESTONE_ID \
  --invoice-signer VENDOR_PAYMENT_KEY_HASH \
  --file 'Invoice - Supplier - Project - YYYYMMDD.pdf' \
  --invoice-number INV-2026-0042 \
  --invoice-date 2026-11-24 --due-date 2026-11-30 \
  --amount 1250.75 --unit lovelace \
  --notes 'October maintenance' --out invoice-request.json
```

The request embeds the invoice bytes, SHA-256, exact target/claims and fresh
portal timestamp/nonce. It does not load a wallet or upload. Prepare when the
operator is ready to sign and return: authorization exchange is limited to two
minutes. A refreshed timestamp/nonce requires a new signature; never reuse a
signed response with a refreshed request. Offline review/sign/verify ignore the
local clock, while online exchange checks current portal time.

## Sign offline

Transfer the request to the airgap. Use the encrypted wallet and correct account;
account 0 is the default. The file contains private invoice data.

```sh
minos invoice review --package invoice-request.json
minos invoice sign --package invoice-request.json \
  --wallet /path/to/wallet.asc --account 0 --out invoice-signed.json
minos invoice verify --package invoice-request.json --signed invoice-signed.json
```

Review network, signer, project/milestone, invoice digest and all accounting claims.
`--expected-request-hash HASH` may pin a reviewed request on `invoice sign`.
One signing run returns the portal's CIP-30 authorization and a separate package
integrity signature (covering ancillary fields such as filename and network).
Neither signature authorizes a Cardano transaction. Return only the signed JSON.

## Upload and confirm online

```sh
minos --network mainnet invoice upload-package \
  --package invoice-request.json --signed invoice-signed.json \
  --out invoice-upload
```

The command verifies both signatures, checks freshness, obtains a private ticket,
POSTs the exact file, and confirms its returned private CID. It reports attached
only after `confirmInvoiceUpload` acknowledges success. No public evidence/IPFS
URL is generated; private storage CID need not equal the evidence UnixFS CID.
Invoice privacy and authorization are enforced by the portal/storage service.

The new directory records each completed stage:

| File | Meaning |
|---|---|
| `request.json` | Request hash and network recorded before requesting the ticket |
| `ticket.json` | Invoice ID, upload capability URL, expiry and confirmation token |
| `uploaded.json` | File stored; invoice target, private CID and confirmation token retained |
| `confirmed.json` | Confirmation acknowledged, or an explicit unknown-outcome marker |

Treat ticket/upload receipts as secrets. Minos does not print their capability
URLs/tokens. An upload/confirmation timeout can have an unknown outcome; inspect
the saved stage and authenticated invoice records before retrying. No automatic
fallback publishes the invoice as public MAF evidence.

If `uploaded.json` exists but confirmation was rejected, interrupted or ambiguous,
retry **only confirmation**, using a fresh result filename:

```sh
minos --network mainnet invoice confirm \
  --receipt invoice-upload/uploaded.json \
  --out invoice-upload/confirmed-retry.json
```

This does not require signing, another ticket or another file upload. If the
upload itself failed/was ambiguous, keep the ticket and check the portal/storage
outcome before starting another upload; Minos does not blindly replay tickets.
An authorization rejected before ticket creation needs fresh preparation/signing.
Output reservations may leave an empty file on validation/network failure; inspect
and preserve it, then use a new destination. Never overwrite a valid artifact.

## Unlock, list and download

Invoice metadata and downloads need a separate, project-scoped wallet session.
Prepare online, sign offline using the same `invoice sign` command, then exchange
it online within two minutes:

```sh
minos --network mainnet invoice prepare-unlock \
  --instance INSTANCE_POLICY_ID --project PROJECT_ID \
  --invoice-signer VENDOR_PAYMENT_KEY_HASH --out invoice-unlock-request.json

# On the airgap:
minos invoice sign --package invoice-unlock-request.json \
  --wallet /path/to/wallet.asc --out invoice-unlock-signed.json

# Back online:
minos --network mainnet invoice unlock \
  --package invoice-unlock-request.json --signed invoice-unlock-signed.json \
  --out invoice-session.json
minos --network mainnet invoice list \
  --session invoice-session.json --out invoice-list.json
minos --network mainnet invoice download \
  --session invoice-session.json --milestone MILESTONE_ID --invoice INVOICE_ID \
  --out downloaded-invoice.pdf
```

The session file holds a short-lived secret token. Listing writes private metadata
to a file instead of stdout. Download selects the exact milestone/invoice from
that authenticated list, requires Available status, obtains a short-lived HTTPS
link and verifies SHA-256 and reported size before saving any document content.
Wrong-network/expired sessions fail; unlock again when needed. The CLI does not
add a finance-wide session or delete invoices.

## Protocol and validation boundary

The implementation follows the public mainnet GraphQL schema and client bundle
`https://treasury.sundae.fi/entry/index-Bmu7XcAK.js`, observed 2026-11-24.
Upload signatures bind action, instance, project, milestone, SHA-256, invoice
number/date/due date, amount/unit, notes, nonce and timestamp in that order.
Ticket issuance, file upload and confirmation are separate operations. File
transfers require HTTPS with no embedded credentials/fragments, disable redirects,
and bound response size. No contract reference scripts, collateral or on-chain
fee inputs are required for these private invoice operations.

Automated validation uses synthetic invoices and disposable/public test keys with
mock portal/storage endpoints; tests do not submit real invoices.

Live preview acceptance passed on 2026-11-25 using the existing authorized Se7en
preview wallet and a clearly marked synthetic invoice for 50 test ADA. The
prepare/sign/upload/confirm flow attached it to CTW-26-0024 milestone m-8.
A separate signed unlock request enabled listing and download; the downloaded
PDF matched the original SHA-256. No Cardano transaction or withdrawal occurred.
This verifies preview ADA handling; mainnet USDM unit acceptance remains untested.
