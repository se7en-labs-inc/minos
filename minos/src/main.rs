mod coin_select;
mod config;
mod koios;
mod maf;
mod node;
mod portal;
mod scripts;
mod tx;
mod types;
mod uplc;
mod wallet;

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

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Generate a new 24-word mnemonic
    New {
        /// GPG recipient email for encrypted output (implies --out)
        #[arg(long)]
        gpg_recipient: Option<String>,

        /// Write to file (required when --gpg-recipient is set)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Show address and key hash for an account
    Address {
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

        /// Wallet address (for fee inputs and collateral)
        #[arg(long)]
        fee_addr: String,

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

        /// Admin key hash (hex, 28 bytes) — controls all treasury/vendor permissions
        #[arg(long)]
        admin_key: String,

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

        /// Write unsigned tx to this file (default: print to stdout)
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },

    /// Create a vendor UTxO (fund a project with ADA and/or tokens)
    Fund {
        /// Registry token policy ID (instance identifier)
        #[arg(long)]
        instance: String,

        /// Project identifier (arbitrary string, e.g. "CTW-26-0024")
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
    let portal_url = cli
        .portal
        .as_deref()
        .unwrap_or_else(|| cli.network.portal_graphql_url());
    let portal = Portal::new(portal_url);

    let ds: DataSource = if let Some(socket) = cli.node_socket {
        let magic = cli
            .testnet_magic
            .unwrap_or_else(|| cli.network.testnet_magic());
        DataSource::Node(node::LocalNode::new(socket, u64::from(magic)))
    } else {
        DataSource::Koios(Koios::new(cli.network.koios_base_url(), cli.koios_key))
    };

    match cli.command {
        Commands::Instances => cmd_instances(&portal).await,
        Commands::Status { instance } => cmd_status(&portal, &instance).await,
        Commands::Complete {
            instance,
            project,
            milestone,
            description,
            evidence_urls,
            fee_addr,
            out,
        } => {
            cmd_complete(
                &portal,
                &ds,
                &instance,
                &project,
                &milestone,
                &description,
                &evidence_urls,
                &fee_addr,
                out.as_deref(),
                cli.network.network_id(),
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
                &instance,
                &project,
                &admin_key,
                &fee_addr,
                publish_tx.as_deref(),
                vendor_utxo.as_deref(),
                milestones.as_deref(),
                out.as_deref(),
                cli.network.network_id(),
            )
            .await
        }
        Commands::Submit {
            tx,
            label,
            portal_only,
        } => cmd_submit(&portal, &ds, &tx, &label, portal_only).await,
        Commands::Wallet { cmd } => cmd_wallet(&ds, cmd, cli.network.network_id()).await,
        Commands::Publish {
            fee_addr,
            admin_key,
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
            )
            .await
        }
        Commands::Mint {
            fee_addr,
            admin_key,
            ticker,
            decimals,
            amount,
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
                .map(|v| v.quantity as u64)
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

// Minimum ADA change output (1.5 ADA)
const MIN_CHANGE: u64 = 1_500_000;
// Upper-bound fee used only for coin selection; actual fee is calculated from tx size.
const ESTIMATED_FEE: u64 = 2_000_000;
// Higher fee for publish (large reference scripts inflate tx size)
const PUBLISH_FEE: u64 = 3_000_000;

/// Calculate the minimum network fee for a serialized transaction.
///
/// Three components:
/// 1. Size fee:       min_fee_a * tx_bytes + min_fee_b
/// 2. Execution fee:  ceil(price_mem * total_mem + price_step * total_steps)
/// 3. Ref script fee: ref_script_cost_per_byte * total_ref_script_bytes
fn calc_min_fee(
    cbor_hex: &str,
    proto: &koios::ProtocolParams,
    spend_mem: u64,
    spend_steps: u64,
    reward_mem: u64,
    reward_steps: u64,
    ref_script_bytes: u64,
    num_witnesses: u64,
) -> u64 {
    let tx_bytes = hex::decode(cbor_hex).unwrap_or_default();
    // Each vkey witness is ~107 bytes (array(2) + bstr(32 vkey) + bstr(64 sig)).
    // The unsigned tx has no vkey witnesses; add them to get the signed tx size.
    let signed_size = tx_bytes.len() as u64 + num_witnesses * 107;
    let size_fee = proto.min_fee_a * signed_size + proto.min_fee_b;
    let total_mem = spend_mem + reward_mem;
    let total_steps = spend_steps + reward_steps;
    let exec_fee = (proto.price_mem() * total_mem as f64 + proto.price_step() * total_steps as f64)
        .ceil() as u64;
    let ref_fee = proto.ref_script_cost_per_byte() * ref_script_bytes;
    size_fee + exec_fee + ref_fee
}

#[allow(clippy::too_many_arguments)]
async fn cmd_complete(
    portal: &Portal,
    koios: &DataSource,
    instance: &str,
    project: &str,
    milestone: &str,
    description: &str,
    evidence_urls: &[String],
    fee_addr: &str,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    // 1. Fetch instance + project data from portal
    let inst = portal.instance_by_id(instance).await?;

    let proj = inst
        .projects
        .iter()
        .find(|p| p.identifier == project)
        .ok_or_else(|| anyhow::anyhow!("project '{project}' not found in instance"))?;

    let _ms = proj
        .milestones
        .iter()
        .find(|m| m.identifier == milestone)
        .ok_or_else(|| anyhow::anyhow!("milestone '{milestone}' not found in project"))?;

    let output = proj
        .unspent_output
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;

    // 2. Decode vendor datum
    let datum = types::VendorDatum::from_cbor_hex(&output.datum_hex)?;
    let vendor_key_hashes = datum.vendor.key_hashes();
    let num_vendor_keys = vendor_key_hashes.len() as u64;

    // 3. Extract vendor UTxO value
    let vendor_value_lovelace = output
        .value
        .iter()
        .find(|v| v.asset_id.is_empty())
        .map(|v| v.quantity as u64)
        .unwrap_or(0);
    let vendor_value_assets: Vec<(String, u64)> = output
        .value
        .iter()
        .filter(|v| !v.asset_id.is_empty())
        .map(|v| (v.asset_id.clone(), v.quantity as u64))
        .collect();

    // 4. Fetch wallet UTxOs
    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();

    // 5. Fetch tip and protocol params
    let tip = koios.tip().await?;
    let proto = koios.protocol_params().await?;

    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model in protocol params"))?;

    // 6. Coin selection
    let (selected, _) = coin_select::select_inputs(&wallet_utxos, ESTIMATED_FEE + MIN_CHANGE)?;
    let total_fee_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let fee_input_hashes: Vec<&str> = selected.iter().map(|u| u.tx_hash.as_str()).collect();
    let collateral = coin_select::select_collateral(&wallet_utxos, &fee_input_hashes)?;

    // 7. Script reference inputs + registry NFT
    let vendor_ref = inst
        .registry
        .vendor_script
        .script_reference
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("vendor script has no scriptReference"))?;
    let treasury_ref = inst
        .registry
        .treasury_script
        .script_reference
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("treasury script has no scriptReference"))?;
    // Registry NFT is at index 0 of the same publish tx as the script refs
    let registry_utxo_hash = vendor_ref.transaction.clone();
    let registry_utxo_index = 0u64;

    // 8. Validity interval
    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    let invalid_from_slot = tip.abs_slot + 7200;

    // 9. Build params — two passes: first to measure tx size, then with exact fee
    let mut params = tx::complete::CompleteParams {
        vendor_utxo_hash: output.transaction.clone(),
        vendor_utxo_index: output.index,
        vendor_address: output.address.clone(),
        vendor_datum_hex: output.datum_hex.clone(),
        vendor_value_lovelace,
        vendor_value_assets,
        registry_utxo_hash,
        registry_utxo_index,
        vendor_script_ref_hash: vendor_ref.transaction.clone(),
        vendor_script_ref_index: vendor_ref.index,
        treasury_script_ref_hash: treasury_ref.transaction.clone(),
        treasury_script_ref_index: treasury_ref.index,
        vendor_key_hashes,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        change_lovelace: total_fee_selected.saturating_sub(ESTIMATED_FEE),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        project_id: project.to_string(),
        milestone_id: milestone.to_string(),
        description: description.to_string(),
        evidence_urls: evidence_urls.to_vec(),
        network_id,
    };
    // Fetch ref script sizes for accurate fee calculation
    let ref_utxos = koios
        .utxo_info(&[
            (vendor_ref.transaction.clone(), vendor_ref.index),
            (treasury_ref.transaction.clone(), treasury_ref.index),
        ])
        .await
        .unwrap_or_default();
    let ref_script_bytes: u64 = ref_utxos
        .iter()
        .map(|u| u.reference_script_size() as u64)
        .sum();
    eprintln!("ref script bytes: {ref_script_bytes}");

    let draft = tx::complete::build(params.clone())?;
    params.fee = calc_min_fee(
        &draft.cbor_hex,
        &proto,
        3_000_000,
        800_000_000,
        0,
        0,
        ref_script_bytes,
        num_vendor_keys + 1,
    );
    params.change_lovelace = total_fee_selected.saturating_sub(params.fee);
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    let unsigned_tx = tx::complete::build(params)?;
    write_tx(unsigned_tx, out)
}

async fn cmd_withdraw(
    portal: &Portal,
    koios: &DataSource,
    instance: &str,
    project: &str,
    fee_addr: &str,
    dest_addr: &str,
    publish_tx: Option<&str>,
    registry_utxo: Option<&str>,
    upcoming: bool,
    ttl_hours: u64,
    out: Option<&std::path::Path>,
    network_id: u8,
    genesis_offset_secs: u64,
) -> Result<()> {
    // 1. Fetch instance + project data
    let inst = portal.instance_by_id(instance).await?;

    let proj = inst
        .projects
        .iter()
        .find(|p| p.identifier == project)
        .ok_or_else(|| anyhow::anyhow!("project '{project}' not found in instance"))?;

    let output = proj
        .unspent_output
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;

    // 2. Decode datum and find matured payouts
    let datum = types::VendorDatum::from_cbor_hex(&output.datum_hex)?;
    let vendor_key_hashes = datum.vendor.key_hashes();
    let num_vendor_keys = vendor_key_hashes.len() as u64;

    // Extract Script multisig hashes (allowlist_vendor staking validators).
    // Map known hashes to their on-chain reference script UTxOs.
    let staking_script_hashes: Vec<[u8; 28]> = datum
        .vendor
        .script_hashes()
        .into_iter()
        .map(|h| {
            h.try_into()
                .map_err(|_| anyhow::anyhow!("script hash must be 28 bytes"))
        })
        .collect::<Result<Vec<_>>>()?;
    let staking_script_refs: Vec<(String, u64)> = staking_script_hashes
        .iter()
        .map(|h| {
            let h_hex = hex::encode(h);
            match h_hex.as_str() {
                // SundaeSwap allowlist_vendor for Se7en Labs instance
                "326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb" => Ok((
                    "aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8".to_string(),
                    0u64,
                )),
                other => anyhow::bail!(
                    "unknown staking script hash {other}; add its reference UTxO to the table"
                ),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let staking_reward_mem: u64 = 2_500_000 * staking_script_hashes.len() as u64;
    let staking_reward_steps: u64 = 900_000_000 * staking_script_hashes.len() as u64;

    // Sanity check: re-encode the full datum and compare with the portal's datum_hex.
    // If these differ, the round-trip is lossy and the script's datum equality check will fail.
    let roundtrip_hex = hex::encode(datum.to_cbor_bytes());
    if roundtrip_hex != output.datum_hex {
        eprintln!("WARNING: datum round-trip mismatch!");
        eprintln!(
            "  portal  : {}",
            &output.datum_hex[..80.min(output.datum_hex.len())]
        );
        eprintln!(
            "  roundtrip: {}",
            &roundtrip_hex[..80.min(roundtrip_hex.len())]
        );
    } else {
        eprintln!("DEBUG: datum round-trip OK");
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    // Fetch tip for abs_slot; genesis_offset_secs is hardcoded per-network (not derived
    // from tip.block_time - tip.abs_slot, which drifts when block_time = SystemTime::now()).
    let tip = koios.tip().await?;

    // Determine the maturation threshold for this withdraw.
    // Normal mode: only payouts already past their maturation time.
    // --upcoming mode: find the earliest future-maturing Active payout and use that batch,
    //   so the tx can be pre-signed now and submitted by a colleague when it matures.
    let latest_maturation_ms = if upcoming {
        let next = datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation > now_ms)
            .map(|p| p.maturation)
            .min()
            .ok_or_else(|| anyhow::anyhow!("no upcoming Active payouts; omit --upcoming to withdraw already-matured payouts"))?;
        // Include all Active payouts up to and including the earliest upcoming batch.
        datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= next)
            .map(|p| p.maturation)
            .max()
            .unwrap_or(next)
    } else {
        let any_matured = datum
            .payouts
            .iter()
            .any(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= now_ms);
        if !any_matured {
            anyhow::bail!("no matured Active payouts available to withdraw");
        }
        datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= now_ms)
            .map(|p| p.maturation)
            .max()
            .unwrap_or(0)
    };
    // Use the current tip slot as valid_from: the maturation is already in the past,
    // so "now" satisfies maturation_time <= lower_bound more clearly than the maturation slot itself.
    // For --upcoming (pre-signing a future payout), fall back to the maturation slot so the
    // tx is valid when submitted later.
    let valid_from_slot = if upcoming {
        (latest_maturation_ms / 1000 + 1).saturating_sub(genesis_offset_secs)
    } else {
        tip.abs_slot.saturating_sub(120)
    };
    let valid_from_posix_ms = (valid_from_slot + genesis_offset_secs) * 1000;

    eprintln!("DEBUG: now_ms={now_ms} tip.block_time={} tip.abs_slot={} genesis_offset={genesis_offset_secs}",
        tip.block_time, tip.abs_slot);
    eprintln!("DEBUG: latest_maturation_ms={latest_maturation_ms} valid_from_slot={valid_from_slot} valid_from_posix_ms={valid_from_posix_ms}");

    // Re-partition using the exact same predicate the Plutus script applies,
    // so our datum and value split always agrees with what the script enforces.
    let (matured_indexed, remaining_indexed): (
        Vec<(usize, &types::Payout)>,
        Vec<(usize, &types::Payout)>,
    ) = datum.payouts.iter().enumerate().partition(|(_, p)| {
        matches!(p.status, types::PayoutStatus::Active) && p.maturation < valid_from_posix_ms
    });

    let matured: Vec<types::Payout> = matured_indexed
        .iter()
        .map(|(i, _)| datum.payouts[*i].clone())
        .collect();
    let remaining: Vec<types::Payout> = remaining_indexed
        .iter()
        .map(|(i, _)| datum.payouts[*i].clone())
        .collect();

    eprintln!(
        "DEBUG: claiming {} payouts, {} remaining",
        matured.len(),
        remaining.len()
    );
    for p in &matured {
        eprintln!("  CLAIM: maturation={}", p.maturation);
    }
    if let Some(first_rem) = remaining.first() {
        eprintln!("  REMAINING starts at: {}", first_rem.maturation);
    }

    // 3. Calculate matured lovelace + assets
    // AssetBundle policy keys: empty bytes = ADA, otherwise native asset policy.
    let mut matured_lovelace_total: u64 = 0;
    let mut matured_assets_map: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();

    for p in &matured {
        for (policy, assets) in &p.value.0 {
            for (name, qty) in assets {
                if policy.is_empty() && name.is_empty() {
                    matured_lovelace_total += *qty as u64;
                } else {
                    let asset_id = format!("{}{}", hex::encode(policy), hex::encode(name));
                    *matured_assets_map.entry(asset_id).or_insert(0) += *qty as u64;
                }
            }
        }
    }
    let matured_assets: Vec<(String, u64)> = matured_assets_map
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // Ensure at least MIN_ADA on destination output
    const MIN_DEST_LOVELACE: u64 = 2_000_000;
    let matured_lovelace_out = matured_lovelace_total.max(MIN_DEST_LOVELACE);

    // 4. Remaining datum and value
    let remaining_datum_bytes = if remaining.is_empty() {
        None
    } else {
        let updated = types::VendorDatum {
            vendor: datum.vendor.clone(),
            payouts: remaining.clone(),
        };
        let bytes = updated.to_cbor_bytes();
        eprintln!("DEBUG: remaining_datum_hex={}", hex::encode(&bytes));
        Some(bytes)
    };

    let vendor_total_lovelace = output
        .value
        .iter()
        .find(|v| v.asset_id.is_empty())
        .map(|v| v.quantity as u64)
        .unwrap_or(0);

    // remaining_lovelace = vendor ADA minus the datum's matured lovelace (NOT the min-ADA floor).
    // The script checks: remaining_output.lovelace >= vendor_input.lovelace - matured_datum_lovelace.
    // Any min-ADA top-up for the dest output comes from fee inputs via the change calculation below.
    let remaining_lovelace = if remaining.is_empty() {
        0
    } else {
        vendor_total_lovelace.saturating_sub(matured_lovelace_total)
    };

    // Remaining assets: vendor total minus what was paid out to dest
    let remaining_assets: Vec<(String, u64)> = output
        .value
        .iter()
        .filter(|v| !v.asset_id.is_empty())
        .filter_map(|v| {
            let vendor_qty = v.quantity as u64;
            let matured_qty = matured_assets_map.get(&v.asset_id).copied().unwrap_or(0);
            let rem = vendor_qty.saturating_sub(matured_qty);
            if rem > 0 {
                Some((v.asset_id.clone(), rem))
            } else {
                None
            }
        })
        .collect();

    // 5. Fetch wallet UTxOs and protocol params (tip already fetched above)
    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();

    let proto = koios.protocol_params().await?;

    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model in protocol params"))?;

    // 6. Registry NFT UTxO + script ref inputs
    struct WithdrawRefInfo {
        registry_utxo_hash: String,
        registry_utxo_index: u64,
        vendor_script_ref_hash: String,
        vendor_script_ref_index: u64,
        treasury_script_ref_hash: String,
        treasury_script_ref_index: u64,
    }
    // Parse --registry-utxo override (format: "<txhash>#<index>")
    let registry_utxo_override: Option<(String, u64)> = registry_utxo
        .map(|s| {
            let (hash, idx) = s
                .split_once('#')
                .ok_or_else(|| anyhow::anyhow!("--registry-utxo must be <txhash>#<index>"))?;
            let idx: u64 = idx
                .parse()
                .map_err(|_| anyhow::anyhow!("--registry-utxo index must be a number"))?;
            Ok::<_, anyhow::Error>((hash.to_string(), idx))
        })
        .transpose()?;

    let ref_info: WithdrawRefInfo = if let Some(pub_tx) = publish_tx {
        let (reg_hash, reg_idx) = registry_utxo_override
            .clone()
            .unwrap_or_else(|| (pub_tx.to_string(), 0));
        WithdrawRefInfo {
            registry_utxo_hash: reg_hash,
            registry_utxo_index: reg_idx,
            vendor_script_ref_hash: pub_tx.to_string(),
            vendor_script_ref_index: 2,
            treasury_script_ref_hash: pub_tx.to_string(),
            treasury_script_ref_index: 1,
        }
    } else {
        let vendor_ref = inst
            .registry
            .vendor_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("vendor script has no scriptReference"))?;
        let treasury_ref = inst
            .registry
            .treasury_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("treasury script has no scriptReference"))?;
        let (reg_hash, reg_idx) = registry_utxo_override
            .clone()
            .unwrap_or_else(|| (vendor_ref.transaction.clone(), 0));
        WithdrawRefInfo {
            registry_utxo_hash: reg_hash,
            registry_utxo_index: reg_idx,
            vendor_script_ref_hash: vendor_ref.transaction.clone(),
            vendor_script_ref_index: vendor_ref.index,
            treasury_script_ref_hash: treasury_ref.transaction.clone(),
            treasury_script_ref_index: treasury_ref.index,
        }
    };

    // 7. Fetch ref script sizes for accurate fee calculation
    let mut ref_utxo_refs: Vec<(String, u64)> = vec![
        (
            ref_info.vendor_script_ref_hash.clone(),
            ref_info.vendor_script_ref_index,
        ),
        (
            ref_info.treasury_script_ref_hash.clone(),
            ref_info.treasury_script_ref_index,
        ),
    ];
    ref_utxo_refs.extend(staking_script_refs.iter().cloned());
    let ref_utxos = koios.utxo_info(&ref_utxo_refs).await.unwrap_or_default();
    let ref_script_bytes: u64 = ref_utxos
        .iter()
        .map(|u| u.reference_script_size() as u64)
        .sum();
    eprintln!("ref script bytes: {ref_script_bytes}");

    // 8. Coin selection
    // Min-ADA top-up: if matured lovelace < MIN_DEST_LOVELACE, the extra comes from fee inputs.
    let min_ada_top_up = matured_lovelace_out.saturating_sub(matured_lovelace_total);
    let (selected, _) =
        coin_select::select_inputs(&wallet_utxos, ESTIMATED_FEE + MIN_CHANGE + min_ada_top_up)?;
    let total_fee_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let fee_input_hashes: Vec<&str> = selected.iter().map(|u| u.tx_hash.as_str()).collect();
    let collateral = coin_select::select_collateral(&wallet_utxos, &fee_input_hashes)?;

    // 9. Validity interval — computed above from maturation timestamps
    let invalid_from_slot = tip.abs_slot + ttl_hours * 3600;

    // 10. Build params — two passes: first to measure tx size, then with exact fee
    let mut params = tx::withdraw::WithdrawParams {
        vendor_utxo_hash: output.transaction.clone(),
        vendor_utxo_index: output.index,
        vendor_address: output.address.clone(),
        matured_lovelace: matured_lovelace_out,
        matured_assets,
        remaining_datum_bytes,
        remaining_lovelace,
        remaining_assets,
        registry_utxo_hash: ref_info.registry_utxo_hash,
        registry_utxo_index: ref_info.registry_utxo_index,
        vendor_script_ref_hash: ref_info.vendor_script_ref_hash,
        vendor_script_ref_index: ref_info.vendor_script_ref_index,
        treasury_script_ref_hash: ref_info.treasury_script_ref_hash,
        treasury_script_ref_index: ref_info.treasury_script_ref_index,
        vendor_key_hashes,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        change_lovelace: total_fee_selected
            .saturating_sub(ESTIMATED_FEE)
            .saturating_sub(min_ada_top_up),
        dest_addr: dest_addr.to_string(),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        network_id,
        staking_script_hashes,
        staking_script_refs,
        staking_reward_mem,
        staking_reward_steps,
    };
    let draft = tx::withdraw::build(params.clone())?;
    // spend: 2.5M mem / 900M steps (~1.5× headroom over reference 1.99M/640M for 19 payouts)
    // reward: same budget per staking script (allowlist_vendor is similarly complex)
    params.fee = calc_min_fee(
        &draft.cbor_hex,
        &proto,
        2_500_000,
        900_000_000,
        params.staking_reward_mem,
        params.staking_reward_steps,
        ref_script_bytes,
        num_vendor_keys + 1,
    );
    params.change_lovelace = total_fee_selected
        .saturating_sub(params.fee)
        .saturating_sub(min_ada_top_up);
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    let unsigned_tx = tx::withdraw::build(params)?;
    write_tx(unsigned_tx, out)
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

#[allow(clippy::too_many_arguments)]
async fn cmd_adjudicate(
    portal: &Portal,
    koios: &DataSource,
    instance: &str,
    project: &str,
    pause_indices_str: Option<&str>,
    resume_indices_str: Option<&str>,
    admin_key_hex: &str,
    fee_addr: &str,
    publish_tx: Option<&str>,
    vendor_utxo_override: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
    genesis_offset_secs: u64,
) -> Result<()> {
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    // Parse pause / resume index lists
    let parse_indices = |s: Option<&str>| -> Result<std::collections::HashSet<usize>> {
        match s {
            None => Ok(std::collections::HashSet::new()),
            Some(s) if s.is_empty() => Ok(std::collections::HashSet::new()),
            Some(s) => s
                .split(',')
                .map(|t| t.trim().parse::<usize>().context("index must be a number"))
                .collect(),
        }
    };
    let pause_set = parse_indices(pause_indices_str)?;
    let resume_set = parse_indices(resume_indices_str)?;
    anyhow::ensure!(
        !pause_set.is_empty() || !resume_set.is_empty(),
        "must specify at least one --pause-indices or --resume-indices"
    );
    anyhow::ensure!(
        pause_set.is_disjoint(&resume_set),
        "pause and resume index sets must not overlap"
    );

    // -------------------------------------------------------------------------
    // Resolve vendor UTxO (portal or Koios fallback)
    // -------------------------------------------------------------------------
    struct VendorInfo {
        utxo_hash: String,
        utxo_index: u64,
        address: String,
        datum_hex: String,
        lovelace: u64,
        assets: Vec<(String, u64)>,
    }

    let vendor_info: VendorInfo = if let Some(override_ref) = vendor_utxo_override {
        // Direct Koios lookup by UTxO ref
        let (hash, idx) = parse_utxo_ref(override_ref)?;
        let utxo_list = koios.utxo_info(&[(hash.clone(), idx)]).await?;
        let utxo = utxo_list
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO {hash}#{idx} not found on chain"))?;
        let datum_hex = utxo
            .datum_hex()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO has no inline datum"))?;
        let lovelace = utxo.lovelace();
        let assets: Vec<(String, u64)> = utxo
            .asset_list
            .as_ref()
            .map(|al| {
                al.iter()
                    .map(|a| {
                        let qty: u64 = a.quantity.parse().unwrap_or(0);
                        (format!("{}{}", a.policy_id, a.asset_name), qty)
                    })
                    .collect()
            })
            .unwrap_or_default();
        VendorInfo {
            utxo_hash: utxo.tx_hash,
            utxo_index: utxo.tx_index,
            address: utxo.address,
            datum_hex,
            lovelace,
            assets,
        }
    } else {
        // Portal lookup
        let inst = portal.instance_by_id(instance).await?;
        let proj = inst
            .projects
            .iter()
            .find(|p| p.identifier == project)
            .ok_or_else(|| anyhow::anyhow!("project '{project}' not found"))?;
        let output = proj
            .unspent_output
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;
        let lovelace = output
            .value
            .iter()
            .find(|v| v.asset_id.is_empty())
            .map(|v| v.quantity as u64)
            .unwrap_or(0);
        let assets: Vec<(String, u64)> = output
            .value
            .iter()
            .filter(|v| !v.asset_id.is_empty())
            .map(|v| (v.asset_id.clone(), v.quantity as u64))
            .collect();
        VendorInfo {
            utxo_hash: output.transaction.clone(),
            utxo_index: output.index,
            address: output.address.clone(),
            datum_hex: output.datum_hex.clone(),
            lovelace,
            assets,
        }
    };

    // -------------------------------------------------------------------------
    // Resolve registry / script reference UTxOs
    // -------------------------------------------------------------------------
    struct RefInfo {
        registry_hash: String,
        registry_index: u64,
        vendor_script_hash: String,
        vendor_script_index: u64,
        treasury_script_hash: String,
        treasury_script_index: u64,
    }

    let ref_info: RefInfo = if let Some(pub_tx) = publish_tx {
        // publish_tx outputs: #0 = registry NFT, #1 = treasury ref, #2 = vendor ref
        RefInfo {
            registry_hash: pub_tx.to_string(),
            registry_index: 0,
            treasury_script_hash: pub_tx.to_string(),
            treasury_script_index: 1,
            vendor_script_hash: pub_tx.to_string(),
            vendor_script_index: 2,
        }
    } else {
        let inst = portal.instance_by_id(instance).await?;
        let vendor_ref = inst
            .registry
            .vendor_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("vendor script has no scriptReference"))?;
        let treasury_ref = inst
            .registry
            .treasury_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("treasury script has no scriptReference"))?;
        // Publish tx layout: #0 = registry NFT, #1 = treasury ref, #2 = vendor ref
        RefInfo {
            registry_hash: vendor_ref.transaction.clone(),
            registry_index: 0,
            vendor_script_hash: vendor_ref.transaction.clone(),
            vendor_script_index: vendor_ref.index,
            treasury_script_hash: treasury_ref.transaction.clone(),
            treasury_script_index: treasury_ref.index,
        }
    };

    // 2. Decode datum
    let datum = types::VendorDatum::from_cbor_hex(&vendor_info.datum_hex)?;
    let n = datum.payouts.len();
    for &i in &pause_set {
        anyhow::ensure!(i < n, "pause index {i} out of range (payouts len = {n})");
    }
    for &i in &resume_set {
        anyhow::ensure!(i < n, "resume index {i} out of range (payouts len = {n})");
    }

    // 3. Build new statuses
    let new_statuses: Vec<types::PayoutStatus> = datum
        .payouts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if pause_set.contains(&i) {
                types::PayoutStatus::Paused
            } else if resume_set.contains(&i) {
                types::PayoutStatus::Active
            } else {
                p.status.clone()
            }
        })
        .collect();

    // 4. Fetch tip; compute valid_from_slot to allow pausing matured Active payouts.
    //    For any Active payout we want to pause, valid_from_slot must correspond to
    //    a POSIX time ≤ that payout's maturation (so is_entirely_after returns False).
    let tip = koios.tip().await?;
    let invalid_from_slot = tip.abs_slot + 7200;

    // Find the earliest maturation among payouts we're pausing that are currently Active
    let earliest_maturation_ms: Option<u64> = pause_set
        .iter()
        .filter(|&&i| matches!(datum.payouts[i].status, types::PayoutStatus::Active))
        .map(|&i| datum.payouts[i].maturation)
        .min();

    let valid_from_slot = if let Some(mat_ms) = earliest_maturation_ms {
        // Use the slot corresponding to that maturation so is_entirely_after is False.
        // Integer division drops sub-second precision intentionally:
        // slot_posix_ms = (genesis + slot) * 1000 ≤ mat_ms  →  is_entirely_after = False.
        let slot_at_maturation = (mat_ms / 1000).saturating_sub(genesis_offset_secs);
        // 36-hour lower bound from invalid_from_slot (interval must fit within 36h)
        let min_slot = invalid_from_slot.saturating_sub(129_600);
        anyhow::ensure!(
            slot_at_maturation >= min_slot,
            "maturation is more than 36 hours in the past — cannot adjudicate within the interval constraint"
        );
        slot_at_maturation
    } else {
        // No Active payouts being paused (all already Paused, or only resuming)
        tip.abs_slot.saturating_sub(120)
    };

    // 5. Wallet UTxOs and protocol params
    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();
    let proto = koios.protocol_params().await?;
    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model"))?;

    let (selected, _) = coin_select::select_inputs(&wallet_utxos, ESTIMATED_FEE + MIN_CHANGE)?;
    let total_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let fee_input_hashes: Vec<&str> = selected.iter().map(|u| u.tx_hash.as_str()).collect();
    let collateral = coin_select::select_collateral(&wallet_utxos, &fee_input_hashes)?;

    // Fetch ref script sizes before ref_info fields are moved into params
    let ref_utxos = koios
        .utxo_info(&[
            (
                ref_info.vendor_script_hash.clone(),
                ref_info.vendor_script_index,
            ),
            (
                ref_info.treasury_script_hash.clone(),
                ref_info.treasury_script_index,
            ),
        ])
        .await
        .unwrap_or_default();
    let ref_script_bytes: u64 = ref_utxos
        .iter()
        .map(|u| u.reference_script_size() as u64)
        .sum();

    let mut params = tx::adjudicate::AdjudicateParams {
        vendor_utxo_hash: vendor_info.utxo_hash,
        vendor_utxo_index: vendor_info.utxo_index,
        vendor_address: vendor_info.address,
        current_datum: datum,
        new_statuses,
        vendor_lovelace: vendor_info.lovelace,
        vendor_assets: vendor_info.assets,
        registry_utxo_hash: ref_info.registry_hash,
        registry_utxo_index: ref_info.registry_index,
        vendor_script_ref_hash: ref_info.vendor_script_hash,
        vendor_script_ref_index: ref_info.vendor_script_index,
        treasury_script_ref_hash: ref_info.treasury_script_hash,
        treasury_script_ref_index: ref_info.treasury_script_index,
        admin_key_hash,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        change_lovelace: total_selected.saturating_sub(ESTIMATED_FEE),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        network_id,
    };

    let draft = tx::adjudicate::build(params.clone())?;
    params.fee = calc_min_fee(
        &draft.cbor_hex,
        &proto,
        3_000_000,
        800_000_000,
        0,
        0,
        ref_script_bytes,
        2,
    );
    params.change_lovelace = total_selected.saturating_sub(params.fee);
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    let unsigned_tx = tx::adjudicate::build(params)?;
    write_tx(unsigned_tx, out)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_modify(
    portal: &Portal,
    koios: &DataSource,
    instance: &str,
    project: &str,
    admin_key_hex: &str,
    fee_addr: &str,
    publish_tx: Option<&str>,
    vendor_utxo_override: Option<&str>,
    milestones_path: Option<&std::path::Path>,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    // -------------------------------------------------------------------------
    // Resolve vendor UTxO (portal or Koios fallback)
    // -------------------------------------------------------------------------
    struct VendorInfo {
        utxo_hash: String,
        utxo_index: u64,
        address: String,
        datum_hex: String,
        lovelace: u64,
        assets: Vec<(String, u64)>,
    }

    let vendor_info: VendorInfo = if let Some(override_ref) = vendor_utxo_override {
        let (hash, idx) = parse_utxo_ref(override_ref)?;
        let utxo_list = koios.utxo_info(&[(hash.clone(), idx)]).await?;
        let utxo = utxo_list
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO {hash}#{idx} not found on chain"))?;
        let datum_hex = utxo
            .datum_hex()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO has no inline datum"))?;
        let lovelace = utxo.lovelace();
        let assets: Vec<(String, u64)> = utxo
            .asset_list
            .as_ref()
            .map(|al| {
                al.iter()
                    .map(|a| {
                        let qty: u64 = a.quantity.parse().unwrap_or(0);
                        (format!("{}{}", a.policy_id, a.asset_name), qty)
                    })
                    .collect()
            })
            .unwrap_or_default();
        VendorInfo {
            utxo_hash: utxo.tx_hash,
            utxo_index: utxo.tx_index,
            address: utxo.address,
            datum_hex,
            lovelace,
            assets,
        }
    } else {
        let inst = portal.instance_by_id(instance).await?;
        let proj = inst
            .projects
            .iter()
            .find(|p| p.identifier == project)
            .ok_or_else(|| anyhow::anyhow!("project '{project}' not found"))?;
        let output = proj
            .unspent_output
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;
        let lovelace = output
            .value
            .iter()
            .find(|v| v.asset_id.is_empty())
            .map(|v| v.quantity as u64)
            .unwrap_or(0);
        let assets: Vec<(String, u64)> = output
            .value
            .iter()
            .filter(|v| !v.asset_id.is_empty())
            .map(|v| (v.asset_id.clone(), v.quantity as u64))
            .collect();
        VendorInfo {
            utxo_hash: output.transaction.clone(),
            utxo_index: output.index,
            address: output.address.clone(),
            datum_hex: output.datum_hex.clone(),
            lovelace,
            assets,
        }
    };

    // -------------------------------------------------------------------------
    // Resolve registry / script reference UTxOs
    // -------------------------------------------------------------------------
    struct RefInfo {
        registry_hash: String,
        registry_index: u64,
        vendor_script_hash: String,
        vendor_script_index: u64,
        treasury_script_hash: String,
        treasury_script_index: u64,
    }

    let ref_info: RefInfo = if let Some(pub_tx) = publish_tx {
        // publish_tx outputs: #0 = registry NFT, #1 = treasury ref, #2 = vendor ref
        RefInfo {
            registry_hash: pub_tx.to_string(),
            registry_index: 0,
            treasury_script_hash: pub_tx.to_string(),
            treasury_script_index: 1,
            vendor_script_hash: pub_tx.to_string(),
            vendor_script_index: 2,
        }
    } else {
        let inst = portal.instance_by_id(instance).await?;
        let vendor_ref = inst
            .registry
            .vendor_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("vendor script has no scriptReference"))?;
        let treasury_ref = inst
            .registry
            .treasury_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("treasury script has no scriptReference"))?;
        // Publish tx layout: #0 = registry NFT, #1 = treasury ref, #2 = vendor ref
        RefInfo {
            registry_hash: vendor_ref.transaction.clone(),
            registry_index: 0,
            vendor_script_hash: vendor_ref.transaction.clone(),
            vendor_script_index: vendor_ref.index,
            treasury_script_hash: treasury_ref.transaction.clone(),
            treasury_script_index: treasury_ref.index,
        }
    };

    // 2. Decode current datum
    let datum = types::VendorDatum::from_cbor_hex(&vendor_info.datum_hex)?;

    // 3. Build corrected datum.
    let corrected_payouts: Vec<types::Payout> = if let Some(ms_path) = milestones_path {
        // Replace payouts wholesale from milestones JSON.
        // Infer token policy+name from the first non-empty payout in the existing datum.
        let milestone_specs: Vec<MilestoneJson> =
            serde_json::from_str(&std::fs::read_to_string(ms_path)?).context("milestones JSON")?;
        anyhow::ensure!(!milestone_specs.is_empty(), "milestones list is empty");

        // Find (policy_bytes, name_bytes) from existing datum (skip the b"" zero-ADA policy).
        let token_info: Option<(Vec<u8>, Vec<u8>)> = datum.payouts.iter().find_map(|p| {
            p.value.0.iter().find_map(|(policy, assets)| {
                if policy.is_empty() {
                    return None;
                }
                assets.iter().find_map(|(name, &qty)| {
                    if qty != 0 {
                        Some((policy.clone(), name.clone()))
                    } else {
                        None
                    }
                })
            })
        });
        let (token_policy, token_name) = token_info.ok_or_else(|| {
            anyhow::anyhow!("could not infer token policy/name from existing datum")
        })?;

        milestone_specs
            .into_iter()
            .map(|m| {
                let mut inner: std::collections::BTreeMap<
                    Vec<u8>,
                    std::collections::BTreeMap<Vec<u8>, i64>,
                > = std::collections::BTreeMap::new();
                if m.token_amount > 0 {
                    inner
                        .entry(token_policy.clone())
                        .or_default()
                        .insert(token_name.clone(), m.token_amount as i64);
                }
                if m.lovelace > 0 {
                    inner
                        .entry(vec![])
                        .or_default()
                        .insert(vec![], m.lovelace as i64);
                }
                types::Payout {
                    maturation: m.maturation_ms,
                    value: types::AssetBundle(inner),
                    status: types::PayoutStatus::Active,
                }
            })
            .collect()
    } else {
        // Strip zero-quantity entries and reset statuses to Active.
        datum
            .payouts
            .iter()
            .map(|p| {
                let cleaned_inner: std::collections::BTreeMap<
                    Vec<u8>,
                    std::collections::BTreeMap<Vec<u8>, i64>,
                > = p
                    .value
                    .0
                    .iter()
                    .filter_map(|(policy, assets)| {
                        let non_zero: std::collections::BTreeMap<Vec<u8>, i64> = assets
                            .iter()
                            .filter(|(_, &qty)| qty != 0)
                            .map(|(k, &v)| (k.clone(), v))
                            .collect();
                        if non_zero.is_empty() {
                            None
                        } else {
                            Some((policy.clone(), non_zero))
                        }
                    })
                    .collect();
                types::Payout {
                    maturation: p.maturation,
                    value: types::AssetBundle(cleaned_inner),
                    status: types::PayoutStatus::Active,
                }
            })
            .collect()
    };

    let new_datum = types::VendorDatum {
        vendor: datum.vendor.clone(),
        payouts: corrected_payouts,
    };
    let new_datum_bytes = new_datum.to_cbor_bytes();

    if milestones_path.is_some() {
        eprintln!(
            "New datum: {} payouts from milestones JSON, all Active",
            new_datum.payouts.len()
        );
    } else {
        eprintln!(
            "Corrected datum: {} payouts, all Active, zero-qty entries stripped",
            new_datum.payouts.len()
        );
    }

    // 4. Fetch tip and wallet UTxOs
    let tip = koios.tip().await?;
    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    let invalid_from_slot = tip.abs_slot + 7200;

    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();
    let proto = koios.protocol_params().await?;
    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model"))?;

    let (selected, _) = coin_select::select_inputs(&wallet_utxos, ESTIMATED_FEE + MIN_CHANGE)?;
    let total_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let fee_input_hashes: Vec<&str> = selected.iter().map(|u| u.tx_hash.as_str()).collect();
    let collateral = coin_select::select_collateral(&wallet_utxos, &fee_input_hashes)?;

    // Fetch ref script sizes before ref_info fields are moved into params
    let ref_utxos = koios
        .utxo_info(&[
            (
                ref_info.vendor_script_hash.clone(),
                ref_info.vendor_script_index,
            ),
            (
                ref_info.treasury_script_hash.clone(),
                ref_info.treasury_script_index,
            ),
        ])
        .await
        .unwrap_or_default();
    let ref_script_bytes: u64 = ref_utxos
        .iter()
        .map(|u| u.reference_script_size() as u64)
        .sum();

    let mut params = tx::modify::ModifyParams {
        vendor_utxo_hash: vendor_info.utxo_hash,
        vendor_utxo_index: vendor_info.utxo_index,
        vendor_address: vendor_info.address,
        new_datum_bytes,
        vendor_lovelace: vendor_info.lovelace,
        vendor_assets: vendor_info.assets,
        registry_utxo_hash: ref_info.registry_hash,
        registry_utxo_index: ref_info.registry_index,
        vendor_script_ref_hash: ref_info.vendor_script_hash,
        vendor_script_ref_index: ref_info.vendor_script_index,
        treasury_script_ref_hash: ref_info.treasury_script_hash,
        treasury_script_ref_index: ref_info.treasury_script_index,
        admin_key_hash,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        change_lovelace: total_selected.saturating_sub(ESTIMATED_FEE),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        network_id,
    };

    let draft = tx::modify::build(params.clone())?;
    params.fee = calc_min_fee(
        &draft.cbor_hex,
        &proto,
        3_000_000,
        800_000_000,
        0,
        0,
        ref_script_bytes,
        2,
    );
    params.change_lovelace = total_selected.saturating_sub(params.fee);
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    let unsigned_tx = tx::modify::build(params)?;
    write_tx(unsigned_tx, out)
}

async fn cmd_wallet(koios: &DataSource, cmd: WalletCmd, network_id: u8) -> Result<()> {
    match cmd {
        WalletCmd::New { gpg_recipient, out } => {
            if gpg_recipient.is_some() && out.is_none() {
                anyhow::bail!("--gpg-recipient requires --out <file>");
            }
            // Generate 32 bytes (256 bits) of OS entropy → 24 BIP39 words
            let mut entropy = [0u8; 32];
            {
                use std::io::Read;
                std::fs::File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut entropy))
                    .context("read /dev/urandom")?;
            }
            let mnemonic = bip39::Mnemonic::from_entropy(&entropy)
                .map_err(|e| anyhow::anyhow!("mnemonic generate: {e}"))?;
            let phrase = mnemonic.words().collect::<Vec<_>>().join(" ");
            match &out {
                Some(path) => {
                    wallet::save_mnemonic(&mnemonic, path, gpg_recipient.as_deref())?;
                    println!("Saved to {}", path.display());
                    // Always print the phrase to stdout too so user can verify/backup
                    println!("Mnemonic: {phrase}");
                }
                None => println!("{phrase}"),
            }
        }

        WalletCmd::Address { wallet, account } => {
            let mnemonic = load_wallet(wallet.as_deref())?;
            let addr = wallet::enterprise_address(&mnemonic, account, network_id);
            let key_hash = wallet::payment_key_hash(&mnemonic, account);
            println!("Account:   {account}");
            println!(
                "Address:   {}",
                addr.to_bech32().unwrap_or_else(|_| addr.to_hex())
            );
            println!("Key hash:  {}", hex::encode(*key_hash));
        }

        WalletCmd::Submit { tx } => cmd_wallet_submit(koios, &tx).await?,

        WalletCmd::Sign {
            tx,
            wallet,
            account,
            out,
        } => {
            let mnemonic = load_wallet(wallet.as_deref())?;
            let tx_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&tx)?)?;
            let cbor_hex = tx_json["cborHex"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing cborHex in tx file"))?;
            let tx_cbor = hex::decode(cbor_hex).context("cborHex decode")?;

            let body_hash = tx::sign::tx_body_hash(&tx_cbor)?;
            let (vkey, signature) = wallet::sign_tx_body(&mnemonic, account, &body_hash);

            let (signed_cbor, _hash) = tx::sign::sign_tx(&tx_cbor, &vkey, &signature)?;

            let description = tx_json["description"].as_str().unwrap_or("").to_string();
            let signed_json = serde_json::json!({
                "type": "Witnessed Tx ConwayEra",
                "description": description,
                "cborHex": hex::encode(&signed_cbor),
            });
            let json_str = serde_json::to_string_pretty(&signed_json)?;
            match &out {
                Some(path) => {
                    std::fs::write(path, &json_str)?;
                    println!("Signed tx written to {}", path.display());
                }
                None => println!("{json_str}"),
            }
        }
    }
    Ok(())
}

/// Load a mnemonic from a file path, or prompt reading from stdin.
fn load_wallet(path: Option<&std::path::Path>) -> Result<bip39::Mnemonic> {
    match path {
        Some(p) => wallet::load_mnemonic(p),
        None => {
            // Read from stdin (useful for piping)
            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .context("read mnemonic from stdin")?;
            bip39::Mnemonic::parse(line.trim())
                .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn cmd_publish(
    koios: &DataSource,
    fee_addr: &str,
    admin_key_hex: &str,
    expiry: &str,
    payout_upperbound: &str,
    vendor_expiry: &str,
    label: &str,
    description: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    // Parse admin key hash
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key must be hex")?;
    anyhow::ensure!(
        admin_key_hash.len() == 28,
        "admin_key must be 28 bytes (56 hex chars)"
    );

    // Parse timestamps
    let expiration_ms = parse_time_ms(expiry)?;
    let payout_upperbound_ms = parse_time_ms(payout_upperbound)?;
    let vendor_expiration_ms = parse_time_ms(vendor_expiry)?;

    // Fetch wallet UTxOs, tip, protocol params
    let utxos = koios.address_utxos(fee_addr).await?;
    anyhow::ensure!(!utxos.is_empty(), "no UTxOs found at {fee_addr}");

    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();

    let tip = koios.tip().await?;
    let proto = koios.protocol_params().await?;
    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model in protocol params"))?;

    // Coin selection: registry NFT (2 ADA) + treasury ref (25 ADA) + vendor ref (36 ADA) + fee + change
    let fixed_outputs = 2_000_000 + 25_000_000 + 36_000_000; // 63 ADA
    let total_target = fixed_outputs + PUBLISH_FEE + MIN_CHANGE;
    let (selected, _) = coin_select::select_inputs(&wallet_utxos, total_target)?;
    let total_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let change_lovelace = total_selected
        .saturating_sub(fixed_outputs)
        .saturating_sub(PUBLISH_FEE);
    anyhow::ensure!(!selected.is_empty(), "coin selection returned no inputs");

    // The first selected input is the seed UTxO.
    let seed = &selected[0];
    let extra_fee_inputs: Vec<coin_select::WalletUtxo> =
        selected[1..].iter().map(|u| (*u).clone()).collect();

    let fee_input_hashes: Vec<&str> = selected.iter().map(|u| u.tx_hash.as_str()).collect();
    let collateral = coin_select::select_collateral(&wallet_utxos, &fee_input_hashes)?;

    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    // 2-hour window keeps slot-to-time translation within the era horizon for Plutus
    let invalid_from_slot = tip.abs_slot + 7200;

    let params = tx::publish::PublishParams {
        seed_utxo_hash: seed.tx_hash.clone(),
        seed_utxo_index: seed.output_index,
        admin_key_hash,
        expiration_ms,
        payout_upperbound_ms,
        vendor_expiration_ms,
        extra_fee_inputs,
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: PUBLISH_FEE,
        change_lovelace,
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        network_id,
        label: label.to_string(),
        description: description.map(|s| s.to_string()),
    };

    let unsigned_tx = tx::publish::build(params)?;
    write_tx(unsigned_tx, out)
}

// ---------------------------------------------------------------------------
// Milestone JSON schema (for --milestones file in fund command)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct MilestoneJson {
    id: String,
    label: String,
    description: Option<String>,
    maturation_ms: u64,
    #[serde(default)]
    lovelace: u64,
    #[serde(default)]
    token_amount: u64,
}

#[allow(clippy::too_many_arguments)]
async fn cmd_fund(
    portal: &Portal,
    koios: &DataSource,
    instance: &str,
    project: &str,
    label: &str,
    description: &str,
    vendor_label: &str,
    milestones_path: &std::path::Path,
    fee_addr: &str,
    admin_key_hex: &str,
    token_policy_hex: Option<&str>,
    token_name_hex: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    // Parse inputs
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    let milestone_specs: Vec<MilestoneJson> =
        serde_json::from_str(&std::fs::read_to_string(milestones_path)?)
            .context("milestones JSON")?;
    anyhow::ensure!(!milestone_specs.is_empty(), "milestones list is empty");

    let token_policy: Option<Vec<u8>> = token_policy_hex
        .map(|h| hex::decode(h.trim()).context("token_policy hex"))
        .transpose()?;
    if let Some(ref p) = token_policy {
        anyhow::ensure!(p.len() == 28, "token_policy must be 28 bytes");
    }
    let token_name: Option<Vec<u8>> = token_name_hex
        .map(|h| hex::decode(h.trim()).context("token_name hex"))
        .transpose()?;

    let total_token_needed: u64 = milestone_specs.iter().map(|m| m.token_amount).sum();
    let total_lovelace_needed: u64 = milestone_specs.iter().map(|m| m.lovelace).sum();

    // Fetch portal instance for vendor script hash
    let inst = portal.instance_by_id(instance).await?;
    let vendor_script_hash =
        hex::decode(&inst.registry.vendor_script.script_hash).context("vendor script hash hex")?;
    anyhow::ensure!(
        vendor_script_hash.len() == 28,
        "vendor script hash must be 28 bytes"
    );

    // Fetch wallet UTxOs from Koios
    let utxos = koios.address_utxos(fee_addr).await?;
    anyhow::ensure!(!utxos.is_empty(), "no UTxOs at {fee_addr}");

    // Select inputs: prefer UTxOs with the right tokens, then fill ADA from the rest.
    let mut inputs: Vec<tx::fund::FundInput> = Vec::new();
    let mut total_input_lovelace: u64 = 0;
    let mut total_input_tokens: u64 = 0;

    // First pass: collect token UTxOs
    if total_token_needed > 0 {
        let (policy_hex, name_hex) = match (token_policy_hex, token_name_hex) {
            (Some(p), Some(n)) => (p, n),
            _ => anyhow::bail!("milestones require tokens but --token-policy/--token-name not set"),
        };
        for u in &utxos {
            if total_input_tokens >= total_token_needed {
                break;
            }
            let qty = u
                .asset_list
                .as_ref()
                .and_then(|assets| {
                    assets
                        .iter()
                        .find(|a| a.policy_id == policy_hex && a.asset_name == name_hex)
                })
                .and_then(|a| a.quantity.parse::<u64>().ok())
                .unwrap_or(0);
            if qty > 0 {
                total_input_tokens += qty;
                total_input_lovelace += u.lovelace();
                inputs.push(tx::fund::FundInput {
                    tx_hash: u.tx_hash.clone(),
                    output_index: u.tx_index,
                    lovelace: u.lovelace(),
                    token_amount: qty,
                });
            }
        }
        anyhow::ensure!(
            total_input_tokens >= total_token_needed,
            "insufficient tokens: need {total_token_needed}, wallet has {total_input_tokens}"
        );
    }

    // Second pass: collect additional ADA UTxOs if needed.
    // Use 10 ADA floor for vendor output (min-ADA for token UTxO with large datum).
    let vendor_floor = total_lovelace_needed.max(10_000_000);
    let target_lovelace = vendor_floor + ESTIMATED_FEE + MIN_CHANGE;
    let already_used: std::collections::HashSet<String> =
        inputs.iter().map(|i| i.tx_hash.clone()).collect();
    for u in &utxos {
        if total_input_lovelace >= target_lovelace {
            break;
        }
        if already_used.contains(&u.tx_hash) {
            continue;
        }
        total_input_lovelace += u.lovelace();
        inputs.push(tx::fund::FundInput {
            tx_hash: u.tx_hash.clone(),
            output_index: u.tx_index,
            lovelace: u.lovelace(),
            token_amount: 0,
        });
    }
    anyhow::ensure!(
        total_input_lovelace >= target_lovelace,
        "insufficient ADA: need {} lovelace ({:.2} ADA)",
        target_lovelace,
        target_lovelace as f64 / 1_000_000.0
    );

    // Ensure vendor output has at least 10 ADA (min-ADA for a token-bearing UTxO with large datum).
    const MIN_VENDOR_LOVELACE: u64 = 10_000_000;
    let vendor_lovelace_total = total_lovelace_needed.max(MIN_VENDOR_LOVELACE);

    let change_lovelace = total_input_lovelace
        .saturating_sub(vendor_lovelace_total)
        .saturating_sub(ESTIMATED_FEE);
    let change_tokens = total_input_tokens.saturating_sub(total_token_needed);

    // Collect extra (non-target) native assets from selected inputs so they go back in change.
    let selected_hashes: std::collections::HashSet<String> =
        inputs.iter().map(|i| i.tx_hash.clone()).collect();
    let mut extra_change_assets: Vec<(Vec<u8>, Vec<u8>, u64)> = Vec::new();
    for u in &utxos {
        if !selected_hashes.contains(&u.tx_hash) {
            continue;
        }
        if let Some(assets) = &u.asset_list {
            for a in assets {
                let is_target = token_policy_hex.map(|p| p == a.policy_id).unwrap_or(false)
                    && token_name_hex.map(|n| n == a.asset_name).unwrap_or(false);
                if is_target {
                    continue;
                }
                let qty: u64 = a.quantity.parse().unwrap_or(0);
                if qty > 0 {
                    let policy = hex::decode(&a.policy_id).unwrap_or_default();
                    let name = hex::decode(&a.asset_name).unwrap_or_default();
                    extra_change_assets.push((policy, name, qty));
                }
            }
        }
    }

    let tip = koios.tip().await?;
    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    let invalid_from_slot = tip.abs_slot + 7200;

    let milestones: Vec<tx::fund::MilestoneSpec> = milestone_specs
        .into_iter()
        .map(|m| tx::fund::MilestoneSpec {
            id: m.id,
            label: m.label,
            description: m.description,
            maturation_ms: m.maturation_ms,
            lovelace: m.lovelace,
            token_amount: m.token_amount,
        })
        .collect();

    let params = tx::fund::FundParams {
        instance_id: instance.to_string(),
        vendor_script_hash,
        admin_key_hash,
        project_id: project.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        vendor_label: vendor_label.to_string(),
        milestones,
        token_policy,
        token_name,
        inputs,
        vendor_lovelace_total,
        change_lovelace,
        change_tokens,
        extra_change_assets,
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        valid_from_slot,
        invalid_from_slot,
        network_id,
    };

    let unsigned_tx = tx::fund::build(params)?;
    write_tx(unsigned_tx, out)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_mint(
    koios: &DataSource,
    fee_addr: &str,
    admin_key_hex: &str,
    ticker: &str,
    decimals: u64,
    amount: u64,
    dest_addr: &str,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    let utxos = koios.address_utxos(fee_addr).await?;
    anyhow::ensure!(!utxos.is_empty(), "no UTxOs at {fee_addr}");

    // Token output needs 3 ADA (ref NFT + FTs in same output)
    let target = 3_000_000 + ESTIMATED_FEE + MIN_CHANGE;
    let wallet_utxos: Vec<coin_select::WalletUtxo> =
        utxos.iter().map(coin_select::WalletUtxo::from).collect();
    let (selected, _) = coin_select::select_inputs(&wallet_utxos, target)?;
    let total_selected: u64 = selected.iter().map(|u| u.lovelace).sum();
    let change_lovelace = total_selected
        .saturating_sub(3_000_000)
        .saturating_sub(ESTIMATED_FEE);

    let tip = koios.tip().await?;
    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    let invalid_from_slot = tip.abs_slot + 7200;

    let inputs: Vec<tx::mint::MintInput> = selected
        .iter()
        .map(|u| tx::mint::MintInput {
            tx_hash: u.tx_hash.clone(),
            output_index: u.output_index,
            lovelace: u.lovelace,
        })
        .collect();

    let params = tx::mint::MintParams {
        admin_key_hash,
        ft_amount: amount,
        token_ticker: ticker.to_string(),
        token_decimals: decimals,
        inputs,
        dest_addr: dest_addr.to_string(),
        fee_addr: fee_addr.to_string(),
        fee: ESTIMATED_FEE,
        change_lovelace,
        valid_from_slot,
        invalid_from_slot,
        network_id,
    };

    let result = tx::mint::build(params)?;

    eprintln!("Policy ID:          {}", result.policy_id_hex);
    eprintln!("FT asset name:      {}", result.ft_asset_name_hex);
    eprintln!("Ref NFT asset name: {}", result.ref_nft_asset_name_hex);
    eprintln!(
        "Full FT asset ID:   {}{}",
        result.policy_id_hex, result.ft_asset_name_hex
    );

    write_tx(result.unsigned_tx, out)
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
) -> Result<()> {
    let cbor_hex = read_cbor_hex(tx_path)?;
    if !portal_only {
        let tx_hash = koios.submit_tx(&cbor_hex).await?;
        println!("Submitted: {tx_hash}");
    }
    match portal.new_transaction(label, &cbor_hex).await {
        Ok(_) => println!("Registered with portal."),
        Err(e) => eprintln!("Portal registration skipped: {e}"),
    }
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
