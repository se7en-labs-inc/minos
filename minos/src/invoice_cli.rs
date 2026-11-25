//! CLI for private invoice records; offline commands dispatch before clients.
use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use minos::{
    invoice::{self, Claims, Package, SignedPackage, UnlockPayload},
    portal::Portal,
    wallet,
};
use std::{io::Read, path::PathBuf};

#[derive(Args)]
pub struct Target {
    #[arg(long)]
    instance: String,
    #[arg(long)]
    project: String,
    /// Authorized vendor payment key hash (28-byte hex).
    #[arg(long = "invoice-signer")]
    signer: String,
}

#[derive(Args)]
pub struct PackagePaths {
    #[arg(long)]
    package: PathBuf,
    #[arg(long)]
    signed: PathBuf,
}

#[derive(Subcommand)]
pub enum InvoiceCmd {
    /// Embed an invoice and fresh authorization for offline signing; no upload.
    PreparePackage {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        milestone: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        invoice_number: String,
        #[arg(long)]
        invoice_date: String,
        #[arg(long)]
        due_date: String,
        /// Decimal major-unit amount, up to six decimal places; never auto-converted.
        #[arg(long)]
        amount: String,
        /// Portal unit identifier; ADA uses lovelace with a decimal ADA amount.
        #[arg(long)]
        unit: String,
        #[arg(long, default_value = "")]
        notes: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Prepare an offline signature to unlock this project's private invoices.
    PrepareUnlock {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        out: PathBuf,
    },
    /// Review target, file digest and claims offline; no wallet or network.
    Review {
        #[arg(long)]
        package: PathBuf,
    },
    /// Sign the invoice authorization and package integrity offline with GPG wallet.
    Sign {
        #[arg(long)]
        package: PathBuf,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long, default_value_t = 0)]
        account: u32,
        #[arg(long)]
        expected_request_hash: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify returned invoice authorization and package signatures offline.
    Verify {
        #[command(flatten)]
        paths: PackagePaths,
    },
    /// Upload privately and confirm attachment; save staged receipts in a new directory.
    UploadPackage {
        #[command(flatten)]
        paths: PackagePaths,
        #[arg(long)]
        out: PathBuf,
    },
    /// Retry confirmation from uploaded.json without signing or uploading again.
    Confirm {
        #[arg(long)]
        receipt: PathBuf,
        /// New confirmation result path; do not overwrite an earlier unknown outcome.
        #[arg(long)]
        out: PathBuf,
    },
    /// Exchange a signed unlock package for a private project session file.
    Unlock {
        #[command(flatten)]
        paths: PackagePaths,
        #[arg(long)]
        out: PathBuf,
    },
    /// Save authenticated invoice metadata in a private file.
    List {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Download and SHA-256 verify one private invoice; never print its capability URL.
    Download {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        milestone: String,
        #[arg(long)]
        invoice: String,
        #[arg(long)]
        out: PathBuf,
    },
}

impl InvoiceCmd {
    pub fn is_offline(&self) -> bool {
        matches!(
            self,
            Self::Review { .. } | Self::Sign { .. } | Self::Verify { .. }
        )
    }
}

fn packages(paths: &PackagePaths) -> Result<(Package, SignedPackage)> {
    Ok((
        invoice::read_private(&paths.package)?,
        invoice::read_private(&paths.signed)?,
    ))
}

pub fn offline(command: InvoiceCmd) -> Result<()> {
    match command {
        InvoiceCmd::Review { package } => {
            let package: Package = invoice::read_private(&package)?;
            println!("{}", serde_json::to_string_pretty(&package.review()?)?);
        }
        InvoiceCmd::Sign {
            package,
            wallet: path,
            account,
            expected_request_hash,
            out,
        } => {
            let package: Package = invoice::read_private(&package)?;
            package.validate()?;
            if let Some(expected) = expected_request_hash {
                ensure!(
                    package.request_hash()? == expected,
                    "invoice request hash mismatch"
                );
            }
            // Reserve the output before accessing the key. Existing files/symlinks fail.
            let mut output = invoice::create_private(&out)?;
            println!("{}", serde_json::to_string_pretty(&package.review()?)?);
            let mnemonic = wallet::load_mnemonic(&path)?;
            ensure!(
                hex::encode(wallet::payment_key_hash(&mnemonic, account)?) == package.signer,
                "wallet/account does not match invoice signer"
            );
            let signed =
                package.sign(|message| wallet::sign_tx_body(&mnemonic, account, message))?;
            use std::io::Write;
            output.write_all(&serde_json::to_vec_pretty(&signed)?)?;
            output.sync_all()?;
            println!("Wrote {}. No transaction signed.", out.display());
        }
        InvoiceCmd::Verify { paths } => {
            let (package, signed) = packages(&paths)?;
            package.verify(&signed)?;
            println!(
                "Verified private invoice signatures: {}",
                package.request_hash()?
            );
        }
        _ => anyhow::bail!("invoice command requires online dispatch"),
    }
    Ok(())
}

fn nonce() -> Result<String> {
    let mut bytes = [0; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex::encode(bytes))
}

pub async fn online(command: InvoiceCmd, portal: &Portal, network: u8) -> Result<()> {
    match command {
        InvoiceCmd::PreparePackage {
            target,
            milestone,
            file,
            invoice_number,
            invoice_date,
            due_date,
            amount,
            unit,
            notes,
            out,
        } => {
            let mut output = invoice::create_private(&out)?;
            let bytes = invoice::read_bounded(&file, invoice::MAX_FILE_BYTES as u64)?;
            let claims = Claims {
                invoice_number,
                invoice_date,
                due_date,
                amount,
                unit,
                notes,
            };
            claims.validate()?;
            let name = file
                .file_name()
                .and_then(|s| s.to_str())
                .context("invalid invoice filename")?
                .to_owned();
            invoice::content_type(&name, &bytes)?;
            let package = Package::upload(
                network,
                target.signer,
                target.instance,
                target.project,
                milestone,
                name,
                bytes,
                claims,
                nonce()?,
                portal.server_timestamp().await?,
            )?;
            use std::io::Write;
            output.write_all(&serde_json::to_vec_pretty(&package)?)?;
            output.sync_all()?;
            println!("Prepared private invoice request {} (hash {}). Sign and return within two minutes; no upload performed.", out.display(), package.request_hash()?);
        }
        InvoiceCmd::PrepareUnlock { target, out } => {
            let mut output = invoice::create_private(&out)?;
            let package = Package::unlock(
                network,
                target.signer,
                UnlockPayload {
                    instance: target.instance,
                    project: target.project,
                    nonce: nonce()?,
                    timestamp: portal.server_timestamp().await?,
                },
            )?;
            use std::io::Write;
            output.write_all(&serde_json::to_vec_pretty(&package)?)?;
            output.sync_all()?;
            println!(
                "Prepared private invoice unlock request {} (hash {}).",
                out.display(),
                package.request_hash()?
            );
        }
        InvoiceCmd::UploadPackage { paths, out } => {
            let (package, signed) = packages(&paths)?;
            invoice::upload(portal, network, &package, &signed, &out).await?;
            println!(
                "Private invoice attached and confirmed. Receipts: {}",
                out.display()
            );
        }
        InvoiceCmd::Confirm { receipt, out } => {
            invoice::confirm(portal, network, &invoice::read_private(&receipt)?, &out).await?;
            println!(
                "Invoice confirmation acknowledged. Result: {}",
                out.display()
            );
        }
        InvoiceCmd::Unlock { paths, out } => {
            let (package, signed) = packages(&paths)?;
            invoice::unlock(portal, network, &package, &signed, &out).await?;
            println!("Saved private invoice session to {}", out.display());
        }
        InvoiceCmd::List { session, out } => {
            let mut file = invoice::create_private(&out)?;
            let result = invoice::list(portal, network, &invoice::read_private(&session)?).await?;
            use std::io::Write;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
            file.sync_all()?;
            println!("Saved private invoice metadata to {}", out.display());
        }
        InvoiceCmd::Download {
            session,
            milestone,
            invoice,
            out,
        } => {
            invoice::download(
                portal,
                network,
                &invoice::read_private(&session)?,
                &milestone,
                &invoice,
                &out,
            )
            .await?;
            println!(
                "Downloaded invoice; SHA-256 verified. File: {}",
                out.display()
            );
        }
        _ => anyhow::bail!("offline invoice command reached online dispatch"),
    }
    Ok(())
}
