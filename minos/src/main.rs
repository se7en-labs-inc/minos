#[path = "workflows/complete.rs"]
mod complete;
use complete::cmd_complete;
#[path = "workflows/withdraw.rs"]
mod withdraw;
use withdraw::cmd_withdraw;
#[path = "workflows/adjudicate.rs"]
mod adjudicate;
use adjudicate::cmd_adjudicate;
#[path = "workflows/modify.rs"]
mod modify;
use modify::cmd_modify;
#[path = "workflows/wallet.rs"]
mod wallet_workflow;
use wallet_workflow::cmd_wallet;
#[path = "workflows/publish.rs"]
mod publish;
use publish::cmd_publish;
#[path = "workflows/fund.rs"]
mod fund;
use fund::cmd_fund;
#[path = "workflows/mint.rs"]
mod mint;
use mint::cmd_mint;

mod airgap_cli;
mod config;
mod maf;
use minos::{coin_select, koios, node, portal, references, tx, types, wallet};

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use config::Network;
use koios::Koios;
use node::DataSource;
use portal::Portal;

#[derive(Parser)]
#[command(name = "minos", about = "Cardano treasury contract tooling")]
struct Cli {
    #[arg(long, default_value = "preview", global = true)]
    network: Network,

    /// Override portal GraphQL URL (default: network's SundaeSwap portal)
    #[arg(long, global = true)]
    portal: Option<String>,

    /// Koios API key (optional; needed for higher rate limits)
    #[arg(long, env = "KOIOS_API_KEY", global = true)]
    koios_key: Option<String>,

    /// Cardano node socket path (direct Pallas connection; bypasses Koios)
    #[arg(long, env = "CARDANO_NODE_SOCKET_PATH", global = true)]
    node_socket: Option<String>,

    /// Network magic for the Pallas handshake (auto-derived from --network if omitted)
    #[arg(long, global = true)]
    testnet_magic: Option<u32>,

    /// Deadline for each node connection or query, in seconds
    #[arg(long, default_value = "30", global = true, value_parser = clap::value_parser!(u64).range(1..))]
    node_timeout_secs: u64,

    /// Verified script location; repeat for each required script.
    #[arg(long, global = true, value_name = "SCRIPT_HASH=TX_HASH#INDEX")]
    script_reference: Vec<references::ScriptReference>,

    /// Vendor signer key hashes to disclose (defaults to all signature leaves).
    /// Select a threshold subset, or declare an actor for script-only permissions.
    #[arg(long, visible_alias = "signer", global = true, value_name = "KEY_HASH")]
    vendor_signer: Vec<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Build an unsigned ADA-only wallet transfer
    Send {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        ada: u64,
        #[arg(long, default_value_t = 1)]
        outputs: u16,
        #[arg(long)]
        out: PathBuf,
    },
    /// Fetch confirmed transaction CBOR over Koios and verify its body hash
    Fetch {
        #[arg(long)]
        hash: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Inspect a transaction's public CBOR fields and body hash without signing
    Inspect {
        #[arg(long)]
        tx: PathBuf,
    },
    /// Generate a new 24-word mnemonic
    New {
        /// GPG recipient email for encrypted output (implies --out)
        #[arg(long)]
        gpg_recipient: Option<String>,

        /// Write to file (required when --gpg-recipient is set)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,

        /// Explicitly print the recovery phrase (required without --out)
        #[arg(long)]
        show_mnemonic: bool,
    },

    /// Show address and key hash for an account
    Address {
        /// Include the account's staking credential in a base address
        #[arg(long)]
        base: bool,
        /// Wallet file (plain text or .gpg). Reads from stdin if omitted.
        #[arg(long, short = 'w')]
        wallet: Option<PathBuf>,

        /// Account index (default: 0)
        #[arg(long, default_value = "0")]
        account: u32,
    },

    /// Sign an unsigned tx with a wallet key
    Sign {
        /// Unsigned tx JSON file (cardano-cli format)
        #[arg(long)]
        tx: PathBuf,

        /// Wallet file (plain text or .gpg)
        #[arg(long, short = 'w')]
        wallet: Option<PathBuf>,

        /// Account index (default: 0)
        #[arg(long, default_value = "0")]
        account: u32,

        /// Write signed tx JSON to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Submit a signed tx to the chain (no portal registration)
    Submit {
        /// Signed tx JSON file (cardano-cli format)
        #[arg(long)]
        tx: PathBuf,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Save the portal's public HTML and client bundle for protocol inspection
    PortalClient {
        #[arg(long)]
        out: PathBuf,
    },
    /// Deposit a project template's funds into its budget treasury
    DepositProject {
        /// Optional change address with the same payment credential and network
        #[arg(long)]
        change_addr: Option<String>,
        /// Recreate an existing exact ADA treasury deposit with initialization metadata
        #[arg(long)]
        reuse_treasury: Option<String>,
        #[arg(long)]
        instance: String,
        #[arg(long)]
        template: PathBuf,
        #[arg(long)]
        fee_addr: String,
        #[arg(long)]
        admin_key: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Clone a public project snapshot with M0 immediate and subsequent timed unlocks
    CloneProject {
        /// Override the vendor display label/address for the preview wallet
        #[arg(long)]
        vendor_label: Option<String>,
        /// Replace every payout with this many whole ADA (preview only)
        #[arg(long, conflicts_with_all = ["token_policy", "token_name"])]
        ada_per_milestone: Option<u64>,
        /// Override the project's identifier while preserving display text
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        source: PathBuf,
        #[arg(long, default_value_t = 15)]
        interval_minutes: u64,
        #[arg(
            long,
            required_unless_present = "ada_per_milestone",
            requires = "token_name"
        )]
        token_policy: Option<String>,
        #[arg(
            long,
            required_unless_present = "ada_per_milestone",
            requires = "token_policy"
        )]
        token_name: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Fund a project from a cloned project template, preserving its metadata
    FundProject {
        /// Separate vendor payment key hash (defaults to admin key)
        #[arg(long)]
        vendor_key: Option<String>,
        /// Additional signing admin payment key hash; repeat for multiple admins
        #[arg(long)]
        co_signer: Vec<String>,
        /// Optional change address with the same payment credential and network
        #[arg(long)]
        change_addr: Option<String>,
        /// Select a specific treasury funding UTxO
        #[arg(long)]
        treasury_utxo: Option<String>,
        #[arg(long)]
        instance: String,
        #[arg(long)]
        template: PathBuf,
        #[arg(long)]
        fee_addr: String,
        #[arg(long)]
        admin_key: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Execute a portal GraphQL query/mutation (no node connection)
    PortalQuery {
        #[arg(long)]
        query: String,
        #[arg(long)]
        variables: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Query wallet UTxOs through the configured HTTP chain backend
    Utxos {
        #[arg(long)]
        address: String,
    },
    /// Review, sign and verify a complete evidence package without network access
    Airgap {
        #[command(subcommand)]
        cmd: airgap_cli::AirgapCmd,
    },
    /// Verify and attach a detached airgap witness without loading any keys
    Assemble {
        #[arg(long)]
        tx: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        expected_body_hash: String,
        #[arg(long)]
        required_signer: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Calculate an evidence CID locally; never uploads or pins files
    Evidence {
        #[command(subcommand)]
        cmd: EvidenceCmd,
    },
    /// Validate a supported unsigned transaction locally without signing or submitting
    Preflight {
        #[arg(long)]
        tx: PathBuf,
    },
    /// List treasury instances known to the portal
    Instances,

    /// Show milestone and payout status for an instance
    Status {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,
    },

    /// Build an unsigned "complete" transaction (attach milestone evidence on-chain)
    Complete {
        /// Known instance publish tx; use vendor reference output #2 if portal omits it
        #[arg(long)]
        publish_tx: Option<String>,
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier
        #[arg(long)]
        project: String,

        /// Milestone identifier
        #[arg(long)]
        milestone: String,

        /// Completion description
        #[arg(long)]
        description: String,

        /// Evidence URL(s) — may be repeated
        #[arg(long = "evidence-url")]
        evidence_urls: Vec<String>,

        /// Display filename/label for each --evidence-url, in the same order
        #[arg(long = "evidence-label", requires = "evidence_urls")]
        evidence_labels: Vec<String>,

        /// Local evidence file; calculate its CIDv1 raw/SHA-256 URL without uploading
        #[arg(long = "evidence-file")]
        evidence_files: Vec<PathBuf>,

        /// Use the portal's live, unreserved sponsored fee inputs
        #[arg(
            long,
            conflicts_with = "fee_addr",
            required_unless_present = "fee_addr"
        )]
        portal_fees: bool,

        /// Wallet address (for fee inputs and collateral)
        #[arg(long, required_unless_present = "portal_fees")]
        fee_addr: Option<String>,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Build an unsigned "withdraw" transaction (claim matured payouts)
    Withdraw {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier
        #[arg(long)]
        project: String,

        /// Wallet address (for fee inputs and collateral)
        #[arg(long)]
        fee_addr: String,

        /// Destination address for withdrawn funds (defaults to --fee-addr)
        #[arg(long)]
        dest_addr: Option<String>,

        /// Publish tx hash — derives registry/script-ref UTxOs at indices 0/1/2
        /// (use when Koios is unavailable or rate-limited)
        #[arg(long)]
        publish_tx: Option<String>,

        /// Registry NFT UTxO override (<txhash>#<index>).
        /// Use when the registry NFT is NOT at index 0 of the publish tx (e.g. externally deployed instances).
        #[arg(long)]
        registry_utxo: Option<String>,

        /// Include the next upcoming (not-yet-matured) payout batch.
        /// Useful for pre-signing a tx before you travel; pair with --ttl-hours.
        #[arg(long)]
        upcoming: bool,

        /// Transaction TTL in hours from the current tip (default: 2).
        /// When pre-signing for a colleague, use a value like 240 (10 days).
        #[arg(long, default_value = "2")]
        ttl_hours: u64,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Submit a signed transaction to the chain
    Submit {
        /// Signed transaction file (cardano-cli format JSON)
        #[arg(long)]
        tx: PathBuf,

        /// Label for portal registration (treasury txs only)
        #[arg(long)]
        label: String,

        /// Skip chain submission; only register with portal (use if tx is already on-chain)
        #[arg(long)]
        portal_only: bool,

        /// Send witnessed CBOR to the portal's newTransaction submission workflow
        #[arg(long, conflicts_with = "portal_only")]
        via_portal: bool,
    },

    /// Wallet management (key derivation, signing)
    Wallet {
        #[command(subcommand)]
        cmd: WalletCmd,
    },

    /// Deploy a new treasury instance (mint registry NFT, create reference scripts)
    Publish {
        /// Wallet address — provides seed UTxO, fees, and collateral
        #[arg(long)]
        fee_addr: String,

        /// Administrator payment key hash; repeat for a threshold policy (first is txAuthor)
        #[arg(long, required = true)]
        admin_key: Vec<String>,

        /// Number of administrator signatures required by each permission
        #[arg(long, default_value_t = 1)]
        admin_threshold: u64,

        /// Instance expiration (RFC 3339 or POSIX ms)
        #[arg(long)]
        expiry: String,

        /// Payout upper-bound date (RFC 3339 or POSIX ms)
        #[arg(long)]
        payout_upperbound: String,

        /// Vendor expiration (RFC 3339 or POSIX ms)
        #[arg(long)]
        vendor_expiry: String,

        /// Human-readable instance label (embedded in CIP-100 metadata for portal discovery)
        #[arg(long)]
        label: String,

        /// Optional description for the instance
        #[arg(long)]
        description: Option<String>,

        /// Create a new directory containing two unsigned tx files and manifest (default: JSON bundle to stdout)
        #[arg(long, short = 'o', value_name = "DIRECTORY")]
        out: Option<PathBuf>,
    },

    /// Create a vendor UTxO (fund a project with ADA and/or tokens)
    Fund {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier (arbitrary string, e.g. "EXAMPLE-001")
        #[arg(long)]
        project: String,

        /// Project label
        #[arg(long)]
        label: String,

        /// Project description
        #[arg(long)]
        description: String,

        /// Vendor label (shown in portal UI)
        #[arg(long)]
        vendor_label: String,

        /// Milestones JSON file (array of {id, label, description?, maturation_ms, lovelace, token_amount})
        #[arg(long)]
        milestones: PathBuf,

        /// Wallet address (fee inputs)
        #[arg(long)]
        fee_addr: String,

        /// Admin key hash (hex, 28 bytes) — used as the vendor multisig key and txAuthor
        #[arg(long)]
        admin_key: String,

        /// Token policy ID hex (28 bytes) — if milestones include token payouts
        #[arg(long)]
        token_policy: Option<String>,

        /// Token asset name hex — e.g. "0014df105553444d37" for USDM7 FT
        #[arg(long)]
        token_name: Option<String>,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Pause or resume individual payouts (oversight / dispute management)
    Adjudicate {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier
        #[arg(long)]
        project: String,

        /// Comma-separated 0-based payout indices to pause (e.g. "0,1")
        #[arg(long)]
        pause_indices: Option<String>,

        /// Comma-separated 0-based payout indices to resume (e.g. "2,3")
        #[arg(long)]
        resume_indices: Option<String>,

        /// Admin key hash (hex, 28 bytes) — pause / resume permission key
        #[arg(long)]
        admin_key: String,

        /// Wallet address (fee inputs and collateral)
        #[arg(long)]
        fee_addr: String,

        /// Publish tx hash — when set, derives registry/script-ref UTxOs at indices 0/1/2
        /// (use when the portal hasn't indexed the instance yet)
        #[arg(long)]
        publish_tx: Option<String>,

        /// Vendor UTxO override: txhash#index (bypasses portal vendor UTxO lookup)
        #[arg(long)]
        vendor_utxo: Option<String>,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Fix a vendor datum (rewrite payouts to strip malformed zero-quantity entries)
    Modify {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier
        #[arg(long)]
        project: String,

        /// Admin key hash (hex, 28 bytes) — vendor multisig + modify permission key
        #[arg(long)]
        admin_key: String,

        /// Wallet address (fee inputs and collateral)
        #[arg(long)]
        fee_addr: String,

        /// Publish tx hash — when set, derives registry/script-ref UTxOs at indices 0/1/2
        /// (use when the portal hasn't indexed the instance yet)
        #[arg(long)]
        publish_tx: Option<String>,

        /// Vendor UTxO override: txhash#index (bypasses portal vendor UTxO lookup)
        #[arg(long)]
        vendor_utxo: Option<String>,

        /// Replace payouts wholesale from a milestones JSON file (same format as fund --milestones).
        /// Token policy/name are inferred from the existing datum. All statuses set to Active.
        #[arg(long)]
        milestones: Option<PathBuf>,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Mint CIP-68 fungible tokens (e.g. USDM7 for preview testing)
    Mint {
        /// Wallet address (fee inputs)
        #[arg(long)]
        fee_addr: String,

        /// Admin key hash (hex, 28 bytes) — becomes the minting policy key
        #[arg(long)]
        admin_key: String,

        /// Token ticker symbol (e.g. "USDM7")
        #[arg(long)]
        ticker: String,

        /// Number of decimal places (e.g. 6)
        #[arg(long, default_value = "6")]
        decimals: u64,

        /// Amount to mint in base units (e.g. 1000000000 = 1000 USDM7 at 6dp)
        #[arg(long)]
        amount: u64,

        /// Mint more fungible units without creating another CIP-68 reference NFT
        #[arg(long)]
        existing_asset: bool,

        /// Destination address for minted tokens (defaults to --fee-addr)
        #[arg(long)]
        dest_addr: Option<String>,

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Generate a Milestone Acceptance Form document
    Maf {
        #[command(subcommand)]
        cmd: MafCmd,
    },
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Prepare one self-contained transaction/PDF/upload package for offline signing
    PreparePackage {
        #[arg(long)]
        tx: PathBuf,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Upload PDF using both verified returned signatures; write ready tx, never submit
    UploadPackage {
        #[arg(long)]
        package: PathBuf,
        #[arg(long)]
        signed: PathBuf,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Print ipfs://CID using CIDv1 raw/SHA-256/base32 (not a UnixFS import)
    Cid {
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum MafCmd {
    /// Populate MAF from GitHub PR and release history
    Github {
        /// GitHub repository (owner/repo), may be repeated
        #[arg(long = "repo")]
        repos: Vec<String>,

        /// Period start date (inclusive, YYYY-MM-DD)
        #[arg(long)]
        start_date: chrono::NaiveDate,

        /// Period end date (inclusive, YYYY-MM-DD)
        #[arg(long)]
        end_date: chrono::NaiveDate,

        /// Milestone label (e.g. "M2.1 Ongoing Maintenance - September 2026")
        #[arg(long)]
        milestone: String,

        /// Project config file (project name, number, signatures, testing plan, comms)
        #[arg(long, default_value = "project.md")]
        project: PathBuf,

        /// Output .docx file
        #[arg(long, short = 'o', default_value = "MAF.docx")]
        out: PathBuf,
    },

    /// Populate MAF from a monthly report markdown file or URL
    Report {
        /// Monthly report markdown file path or https:// URL
        #[arg(long)]
        report: String,

        /// Milestone label (e.g. "M2.1 Ongoing Maintenance - September 2026")
        #[arg(long)]
        milestone: String,

        /// Project config file (project name, number, signatures, testing plan, comms)
        #[arg(long, default_value = "project.md")]
        project: PathBuf,

        /// Output .docx file
        #[arg(long, short = 'o', default_value = "MAF.docx")]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let vendor_signers = cli
        .vendor_signer
        .iter()
        .map(|value| hex::decode(value.trim()).context("invalid --vendor-signer hex"))
        .collect::<Result<Vec<_>>>()?;
    let mut unique_signers = std::collections::BTreeSet::new();
    for signer in &vendor_signers {
        anyhow::ensure!(
            signer.len() == 28 && unique_signers.insert(signer.clone()),
            "vendor signers must be distinct 28-byte key hashes"
        );
    }
    let (fund_vendor_key, fund_co_signers) = match &cli.command {
        Commands::FundProject {
            vendor_key,
            co_signer,
            ..
        } => (vendor_key.clone(), co_signer.clone()),
        _ => (None, vec![]),
    };
    let deposit_project = matches!(&cli.command, Commands::DepositProject { .. });
    let reuse_treasury = match &cli.command {
        Commands::DepositProject { reuse_treasury, .. } => reuse_treasury.clone(),
        Commands::FundProject { treasury_utxo, .. } => treasury_utxo.clone(),
        _ => None,
    };
    // Dispatch offline work before constructing any portal/node clients.
    if let Commands::Airgap { cmd } = cli.command {
        return airgap_cli::run(cmd);
    }
    anyhow::ensure!(
        cli.testnet_magic
            .is_none_or(|m| m == cli.network.testnet_magic()),
        "--testnet-magic conflicts with --network; network timing and addresses must agree"
    );
    let portal_url = cli
        .portal
        .as_deref()
        .unwrap_or_else(|| cli.network.portal_graphql_url());
    let portal = Portal::new(portal_url);

    let ds: DataSource = if let Some(socket) = cli.node_socket {
        let magic = cli
            .testnet_magic
            .unwrap_or_else(|| cli.network.testnet_magic());
        DataSource::Node(
            node::LocalNode::new(socket, u64::from(magic))
                .with_timeout(std::time::Duration::from_secs(cli.node_timeout_secs)),
        )
    } else {
        DataSource::Koios(Koios::new(cli.network.koios_base_url(), cli.koios_key))
    };

    match cli.command {
        Commands::CloneProject {
            vendor_label,
            ada_per_milestone,
            project_id,
            source,
            interval_minutes,
            token_policy,
            token_name,
            out,
        } => {
            anyhow::ensure!(
                cli.network.network_id() == 0,
                "project schedule cloning is preview-only"
            );
            let snapshot = serde_json::from_slice(&std::fs::read(source)?)?;
            let mut template = if let Some(ada) = ada_per_milestone {
                minos::project_template::clone_project_ada(
                    &snapshot,
                    interval_minutes,
                    Utc::now().timestamp_millis() as u64,
                    ada,
                )?
            } else {
                minos::project_template::clone_project(
                    &snapshot,
                    interval_minutes,
                    Utc::now().timestamp_millis() as u64,
                    token_policy,
                    token_name,
                )?
            };
            if let Some(id) = project_id {
                anyhow::ensure!(!id.trim().is_empty(), "project identifier cannot be empty");
                template.project = id;
            }
            if let Some(label) = vendor_label {
                anyhow::ensure!(!label.trim().is_empty(), "vendor label cannot be empty");
                template.vendor_label = label;
            }
            minos::airgap::write_new(&out, &template)?;
            println!("{}", serde_json::to_string_pretty(&template)?);
            println!(
                "Wrote {}. M0 unlocks immediately; fund promptly.",
                out.display()
            );
            Ok(())
        }
        Commands::FundProject {
            vendor_key: _,
            co_signer: _,
            change_addr,
            treasury_utxo: _,
            instance,
            template,
            fee_addr,
            admin_key,
            out,
        }
        | Commands::DepositProject {
            change_addr,
            reuse_treasury: _,
            instance,
            template,
            fee_addr,
            admin_key,
            out,
        } => {
            let project: minos::project_template::ProjectTemplate =
                serde_json::from_slice(&std::fs::read(&template)?)?;
            cmd_fund(
                &portal,
                &ds,
                &instance,
                &project.project,
                &project.label,
                &project.description,
                &project.vendor_label,
                &template,
                &fee_addr,
                &admin_key,
                project.token_policy.as_deref(),
                project.token_name.as_deref(),
                Some(&out),
                cli.network.network_id(),
                deposit_project,
                reuse_treasury.as_deref(),
                change_addr.as_deref(),
                fund_vendor_key.as_deref(),
                &fund_co_signers,
            )
            .await
        }
        Commands::PortalClient { out } => {
            let root = if cli.network.network_id() == 1 {
                "https://treasury.sundae.fi/"
            } else {
                "https://treasury.preview.sundae.fi/"
            };
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
            let html = client
                .get(root)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let script = html
                .split("<script")
                .find(|s| s.contains("type=\"module\""))
                .and_then(|s| s.split("src=\"").nth(1))
                .and_then(|s| s.split('"').next())
                .context("public module script not found")?;
            let url = reqwest::Url::parse(root)?.join(script)?;
            anyhow::ensure!(url.scheme() == "https", "client script must use HTTPS");
            let source = client
                .get(url.clone())
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            std::fs::create_dir(&out).context("output directory must not already exist")?;
            std::fs::write(out.join("index.html"), html)?;
            std::fs::write(out.join("client.js"), source)?;
            println!("Saved public portal client from {url} to {}", out.display());
            Ok(())
        }
        Commands::PortalQuery {
            query,
            variables,
            out,
        } => {
            let variables = variables
                .map(|p| -> Result<serde_json::Value> {
                    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
                })
                .transpose()?;
            let value = portal.query(&query, variables).await?;
            if let Some(out) = out {
                minos::airgap::write_new(&out, &value)?;
                println!("Wrote {}", out.display());
            } else {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            Ok(())
        }
        Commands::Utxos { address } => {
            for u in ds.address_utxos(&address).await? {
                println!(
                    "{}",
                    serde_json::json!({"tx":u.tx_hash,"index":u.tx_index,
                    "lovelace":u.lovelace(),"assets":u.asset_list})
                );
            }
            Ok(())
        }
        Commands::Airgap { cmd } => airgap_cli::run(cmd),
        Commands::Evidence {
            cmd: EvidenceCmd::PreparePackage { tx, file, out },
        } => {
            anyhow::ensure!(!out.exists(), "output already exists");
            use std::io::Read;
            let mut nonce = [0; 16];
            std::fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
            let metadata = std::fs::metadata(&file)?;
            anyhow::ensure!(metadata.len() <= 10 * 1024 * 1024, "PDF exceeds 10 MiB");
            let package = minos::airgap::Package::prepare(
                read_cbor_hex(&tx)?,
                file.file_name()
                    .and_then(|s| s.to_str())
                    .context("invalid PDF filename")?
                    .to_owned(),
                std::fs::read(&file)?,
                cli.network.network_id(),
                hex::encode(nonce),
                portal.server_timestamp().await?,
            )?;
            minos::airgap::write_new(&out, &package)?;
            println!("{}", serde_json::to_string_pretty(&package.review()?)?);
            println!(
                "Wrote {}. No keys loaded, upload or submission performed.",
                out.display()
            );
            Ok(())
        }
        Commands::Evidence {
            cmd:
                EvidenceCmd::UploadPackage {
                    package,
                    signed,
                    receipt,
                    out,
                },
        } => {
            let package: minos::airgap::Package = serde_json::from_slice(&std::fs::read(package)?)?;
            anyhow::ensure!(
                package.network == cli.network.network_id(),
                "package/CLI network mismatch"
            );
            let signed = serde_json::from_slice(&std::fs::read(signed)?)?;
            minos::airgap::upload_package(&portal, &package, &signed, &receipt, &out).await?;
            println!(
                "PDF uploaded; CID verified. Wrote {} and {}. Transaction not submitted.",
                receipt.display(),
                out.display()
            );
            Ok(())
        }
        Commands::Assemble {
            tx,
            witness,
            expected_body_hash,
            required_signer,
            out,
        } => {
            let hash: [u8; 32] = hex::decode(expected_body_hash)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected 32-byte body hash"))?;
            let signer: [u8; 28] = hex::decode(required_signer)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected 28-byte signer hash"))?;
            let assembled = tx::sign::assemble_witness(
                &hex::decode(read_cbor_hex(&tx)?)?,
                &std::fs::read_to_string(witness)?,
                &hash,
                &signer,
            )?;
            let envelope = serde_json::json!({"type": "Witnessed Tx ConwayEra",
                "description": "Verified airgap witness", "cborHex": hex::encode(assembled)});
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)?;
            file.write_all(serde_json::to_string_pretty(&envelope)?.as_bytes())?;
            println!(
                "Verified and assembled {}: {}",
                out.display(),
                hex::encode(hash)
            );
            Ok(())
        }
        Commands::Preflight { tx } => {
            let cbor = read_cbor_hex(&tx)?;
            let proto = ds.protocol_params().await?;
            let report = minos::preflight::withdrawal(
                &ds,
                &cbor,
                &proto,
                cli.network.genesis_offset_secs(),
                &[],
            )
            .await?;
            println!("Withdrawal preflight passed: {} redeemers, {} memory, {} steps. This is not full ledger validation.", report.redeemers, report.memory, report.steps);
            Ok(())
        }
        Commands::Evidence {
            cmd: EvidenceCmd::Cid { file },
        } => {
            println!("{}", minos::evidence::file_url(&file)?);
            Ok(())
        }
        Commands::Instances => cmd_instances(&portal).await,
        Commands::Status { instance } => cmd_status(&portal, &instance).await,
        Commands::Complete {
            publish_tx,
            instance,
            project,
            milestone,
            description,
            evidence_urls,
            evidence_labels,
            evidence_files,
            portal_fees,
            fee_addr,
            out,
        } => {
            let mut evidence_urls = evidence_urls;
            anyhow::ensure!(
                evidence_labels.is_empty() || evidence_labels.len() == evidence_urls.len(),
                "provide one --evidence-label for each --evidence-url, or omit labels"
            );
            let mut evidence_labels = if evidence_labels.is_empty() {
                vec![String::new(); evidence_urls.len()]
            } else {
                evidence_labels
            };
            for path in evidence_files {
                let url = minos::evidence::file_url(&path)?;
                eprintln!("{}: {url} (CIDv1 raw; file not uploaded)", path.display());
                evidence_urls.push(url);
                evidence_labels.push(
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .context("invalid evidence filename")?
                        .to_owned(),
                );
            }
            cmd_complete(
                &portal,
                &ds,
                &cli.script_reference,
                &vendor_signers,
                &instance,
                &project,
                &milestone,
                &description,
                &evidence_urls,
                &evidence_labels,
                fee_addr.as_deref(),
                portal_fees,
                publish_tx.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
                cli.network.genesis_offset_secs(),
            )
            .await
        }
        Commands::Withdraw {
            instance,
            project,
            fee_addr,
            dest_addr,
            publish_tx,
            registry_utxo,
            upcoming,
            ttl_hours,
            out,
        } => {
            cmd_withdraw(
                &portal,
                &ds,
                &cli.script_reference,
                &vendor_signers,
                &instance,
                &project,
                &fee_addr,
                dest_addr.as_deref().unwrap_or(&fee_addr),
                publish_tx.as_deref(),
                registry_utxo.as_deref(),
                upcoming,
                ttl_hours,
                out.as_deref(),
                cli.network.network_id(),
                cli.network.genesis_offset_secs(),
            )
            .await
        }
        Commands::Adjudicate {
            instance,
            project,
            pause_indices,
            resume_indices,
            admin_key,
            fee_addr,
            publish_tx,
            vendor_utxo,
            out,
        } => {
            cmd_adjudicate(
                &portal,
                &ds,
                &cli.script_reference,
                &vendor_signers,
                &instance,
                &project,
                pause_indices.as_deref(),
                resume_indices.as_deref(),
                &admin_key,
                &fee_addr,
                publish_tx.as_deref(),
                vendor_utxo.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
                cli.network.genesis_offset_secs(),
            )
            .await
        }
        Commands::Modify {
            instance,
            project,
            admin_key,
            fee_addr,
            publish_tx,
            vendor_utxo,
            milestones,
            out,
        } => {
            cmd_modify(
                &portal,
                &ds,
                &cli.script_reference,
                &vendor_signers,
                &instance,
                &project,
                &admin_key,
                &fee_addr,
                publish_tx.as_deref(),
                vendor_utxo.as_deref(),
                milestones.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
                cli.network.genesis_offset_secs(),
            )
            .await
        }
        Commands::Submit {
            tx,
            label,
            portal_only,
            via_portal,
        } => cmd_submit(&portal, &ds, &tx, &label, portal_only, via_portal).await,
        Commands::Wallet { cmd } => cmd_wallet(&ds, cmd, cli.network.network_id()).await,
        Commands::Publish {
            fee_addr,
            admin_key,
            admin_threshold,
            expiry,
            payout_upperbound,
            vendor_expiry,
            label,
            description,
            out,
        } => {
            cmd_publish(
                &ds,
                &fee_addr,
                &admin_key,
                admin_threshold,
                &expiry,
                &payout_upperbound,
                &vendor_expiry,
                &label,
                description.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
            )
            .await
        }
        Commands::Fund {
            instance,
            project,
            label,
            description,
            vendor_label,
            milestones,
            fee_addr,
            admin_key,
            token_policy,
            token_name,
            out,
        } => {
            cmd_fund(
                &portal,
                &ds,
                &instance,
                &project,
                &label,
                &description,
                &vendor_label,
                &milestones,
                &fee_addr,
                &admin_key,
                token_policy.as_deref(),
                token_name.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
                false,
                None,
                None,
                None,
                &[],
            )
            .await
        }
        Commands::Mint {
            fee_addr,
            admin_key,
            ticker,
            decimals,
            amount,
            existing_asset,
            dest_addr,
            out,
        } => {
            let dest = dest_addr.as_deref().unwrap_or(&fee_addr);
            cmd_mint(
                &ds,
                &fee_addr,
                &admin_key,
                &ticker,
                decimals,
                amount,
                existing_asset,
                dest,
                out.as_deref(),
                cli.network.network_id(),
            )
            .await
        }
        Commands::Maf { cmd } => match cmd {
            MafCmd::Github {
                repos,
                start_date,
                end_date,
                milestone,
                project,
                out,
            } => maf::cmd_maf_github(repos, start_date, end_date, milestone, project, out).await,
            MafCmd::Report {
                report,
                milestone,
                project,
                out,
            } => maf::cmd_maf_report(report, milestone, project, out).await,
        },
    }
}

async fn cmd_instances(portal: &Portal) -> Result<()> {
    let instances = portal.instances().await?;
    if instances.is_empty() {
        println!("No instances found.");
        return Ok(());
    }
    for inst in &instances {
        println!("{}", inst.identifier);
        println!("  label:      {}", inst.label);
        println!("  expires:    {}", ms_to_utc(inst.expiration_ms));
        println!(
            "  treasury:   {}",
            inst.registry.treasury_script.script_hash
        );
        println!("  vendor:     {}", inst.registry.vendor_script.script_hash);
        println!();
    }
    Ok(())
}

async fn cmd_status(portal: &Portal, instance_id: &str) -> Result<()> {
    let inst = portal.instance_by_id(instance_id).await?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    println!("Instance: {} ({})", inst.label, inst.identifier);
    println!("Expires:  {}", ms_to_utc(inst.expiration_ms));
    println!();

    if inst.projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    for project in &inst.projects {
        println!("Project: {} — {}", project.identifier, project.label);
        println!("  Vendor: {}", project.vendor_label);

        if let Some(output) = &project.unspent_output {
            let lovelace: u64 = output
                .value
                .iter()
                .find(|v| v.asset_id.is_empty())
                .map(|v| v.quantity)
                .unwrap_or(0);
            println!("  UTxO:   {}#{}", output.transaction, output.index);
            println!("  Value:  {} ADA", lovelace / 1_000_000);
        } else {
            println!("  UTxO:   (fully withdrawn)");
        }

        if project.milestones.is_empty() {
            println!("  (no milestones)");
        } else {
            println!("  Milestones:");
            for m in &project.milestones {
                let matured = m.maturation_ms <= now_ms;
                let maturation_str = ms_to_utc(m.maturation_ms);
                let matured_flag = if matured { "✓ matured" } else { "pending" };
                println!(
                    "    [{}] {} — {} — {} ({})",
                    m.identifier, m.label, m.status, maturation_str, matured_flag
                );
                if !m.completions.is_empty() {
                    println!("      Completions:");
                    for c in &m.completions {
                        println!("        {}", c.description);
                        for e in &c.evidence {
                            println!("          evidence: {}", e.anchor_url);
                        }
                    }
                }
            }
        }
        println!();
    }

    Ok(())
}

/// Parse a "txhash#index" string into (hash, index).
fn parse_utxo_ref(s: &str) -> Result<(String, u64)> {
    let (hash, idx) = s
        .split_once('#')
        .ok_or_else(|| anyhow::anyhow!("expected 'txhash#index', got '{s}'"))?;
    let idx = idx
        .parse::<u64>()
        .context("output index must be a number")?;
    Ok((hash.to_string(), idx))
}

/// Parse a time value as POSIX milliseconds.
/// Accepts: plain integer (ms), RFC 3339 date/time string.
fn parse_time_ms(s: &str) -> Result<u64> {
    let s = s.trim();
    // Try plain integer first
    if let Ok(ms) = s.parse::<u64>() {
        return Ok(ms);
    }
    // Try RFC 3339
    let dt: DateTime<Utc> = s
        .parse()
        .with_context(|| format!("cannot parse '{s}' as POSIX ms or RFC 3339"))?;
    Ok(dt.timestamp_millis() as u64)
}

/// Write unsigned tx JSON to file or stdout.
fn write_tx(tx: tx::UnsignedTx, out: Option<&std::path::Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(&tx)?;
    match out {
        Some(path) => {
            std::fs::write(path, &json)?;
            println!("Wrote unsigned tx to {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

async fn cmd_submit(
    portal: &Portal,
    koios: &DataSource,
    tx_path: &std::path::Path,
    label: &str,
    portal_only: bool,
    via_portal: bool,
) -> Result<()> {
    let cbor_hex = read_cbor_hex(tx_path)?;
    if via_portal {
        portal.submit_transaction(label, &cbor_hex).await?;
        let hash = tx::sign::tx_body_hash(&hex::decode(&cbor_hex)?)?;
        println!(
            "Accepted by portal: {} (chain confirmation pending)",
            hex::encode(hash)
        );
        return Ok(());
    }
    if !portal_only {
        let tx_hash = koios.submit_tx(&cbor_hex).await?;
        println!("Submitted: {tx_hash}");
    }
    portal.register_transaction(label, &cbor_hex).await.context(
        "portal registration failed; registration does not submit transactions; retry with --portal-only"
    )?;
    println!("Registered with portal.");
    Ok(())
}

async fn cmd_wallet_submit(koios: &DataSource, tx_path: &std::path::Path) -> Result<()> {
    let cbor_hex = read_cbor_hex(tx_path)?;
    let tx_hash = koios.submit_tx(&cbor_hex).await?;
    println!("Submitted: {tx_hash}");
    Ok(())
}

fn read_cbor_hex(tx_path: &std::path::Path) -> Result<String> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(tx_path)?)?;
    json["cborHex"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing cborHex in tx file"))
}

fn ms_to_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("{ms}ms"))
}
