mod config;
mod koios;
mod portal;
mod types;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use config::Network;
use portal::Portal;

#[derive(Parser)]
#[command(name = "minos", about = "Cardano treasury contract tooling")]
struct Cli {
    #[arg(long, default_value = "preview", global = true)]
    network: Network,

    /// Koios API key (optional; needed for higher rate limits)
    #[arg(long, env = "KOIOS_API_KEY", global = true)]
    koios_key: Option<String>,

    #[command(subcommand)]
    command: Commands,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let portal = Portal::new(cli.network.portal_graphql_url());

    match cli.command {
        Commands::Instances => cmd_instances(&portal).await,
        Commands::Status { instance } => cmd_status(&portal, &instance).await,
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
        println!(
            "  vendor:     {}",
            inst.registry.vendor_script.script_hash
        );
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

fn ms_to_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("{ms}ms"))
}
