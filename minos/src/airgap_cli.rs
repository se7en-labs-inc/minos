//! Offline-only entrypoint: no network clients, server time queries or submission.
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use minos::{
    airgap::{self, Package, SignedPackage},
    wallet,
};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AirgapCmd {
    Review {
        #[arg(long)]
        package: PathBuf,
    },
    Sign {
        #[arg(long)]
        package: PathBuf,
        /// Mnemonic wallet; .asc/.gpg is decrypted by GPG on this offline machine.
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long, default_value_t = 0)]
        account: u32,
        /// Optionally pin the package hash, for scripted signing workflows.
        #[arg(long)]
        expected_request_hash: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    Verify {
        #[arg(long)]
        package: PathBuf,
        #[arg(long)]
        signed: PathBuf,
    },
}

fn read(path: &std::path::Path) -> Result<Package> {
    serde_json::from_slice(&std::fs::read(path)?).context("read package")
}

pub fn run(command: AirgapCmd) -> Result<()> {
    match command {
        AirgapCmd::Review { package } => {
            let package = read(&package)?;
            package.validate_offline()?;
            println!("{}", serde_json::to_string_pretty(&package.review()?)?);
        }
        AirgapCmd::Sign {
            package,
            wallet: path,
            account,
            expected_request_hash,
            out,
        } => {
            let package = read(&package)?;
            package.validate_offline()?;
            if let Some(expected) = expected_request_hash {
                ensure!(
                    package.request_hash()? == expected,
                    "reviewed request hash mismatch"
                );
            }
            ensure!(!out.exists(), "output already exists");
            println!("{}", serde_json::to_string_pretty(&package.review()?)?);
            let mnemonic = wallet::load_mnemonic(&path)?;
            ensure!(
                hex::encode(wallet::payment_key_hash(&mnemonic, account)?) == package.signer,
                "wallet/account does not match vendor signer"
            );
            let signed = package
                .sign_offline(|message| wallet::sign_tx_body(&mnemonic, account, message))?;
            airgap::write_new(&out, &signed)?;
            println!("Wrote both vendor signatures to {}", out.display());
        }
        AirgapCmd::Verify { package, signed } => {
            let package = read(&package)?;
            let signed: SignedPackage = serde_json::from_slice(&std::fs::read(signed)?)?;
            package.verify_offline(&signed)?;
            println!(
                "Verified transaction witness and upload authorization: {}",
                package.body_hash
            );
        }
    }
    Ok(())
}
