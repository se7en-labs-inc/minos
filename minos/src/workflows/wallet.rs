//! wallet command workflow, independent of CLI argument parsing.
use super::*;

pub(super) async fn cmd_wallet(koios: &DataSource, cmd: WalletCmd, network_id: u8) -> Result<()> {
    match cmd {
        WalletCmd::New {
            gpg_recipient,
            out,
            show_mnemonic,
        } => {
            if gpg_recipient.is_some() && out.is_none() {
                anyhow::bail!("--gpg-recipient requires --out <file>");
            }
            anyhow::ensure!(
                out.is_some() || show_mnemonic,
                "wallet new requires --out <file> or explicit --show-mnemonic"
            );
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
            if let Some(path) = &out {
                wallet::save_mnemonic(&mnemonic, path, gpg_recipient.as_deref())?;
                println!("Saved to {}", path.display());
            }
            if show_mnemonic {
                println!("{phrase}");
            }
        }

        WalletCmd::Address {
            wallet,
            account,
            base,
        } => {
            let mnemonic = load_wallet(wallet.as_deref())?;
            let addr = if base {
                wallet::base_address(&mnemonic, account, network_id)?
            } else {
                wallet::enterprise_address(&mnemonic, account, network_id)?
            };
            let key_hash = wallet::payment_key_hash(&mnemonic, account)?;
            println!("Account:   {account}");
            println!(
                "Address:   {}",
                addr.to_bech32().unwrap_or_else(|_| addr.to_hex())
            );
            println!("Key hash:  {}", hex::encode(*key_hash));
        }

        WalletCmd::Send {
            from,
            to,
            ada,
            outputs,
            out,
        } => {
            use pallas_crypto::hash::Hash;
            use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
            let source = pallas_addresses::Address::from_bech32(&from)?;
            let destination = pallas_addresses::Address::from_bech32(&to)?;
            let source_bytes = source.to_vec();
            let destination_bytes = destination.to_vec();
            anyhow::ensure!(
                source_bytes.len() >= 29 && matches!(source_bytes[0] >> 4, 0 | 2 | 6),
                "source must have a payment key credential"
            );
            anyhow::ensure!(
                source_bytes[0] & 15 == network_id && destination_bytes[0] & 15 == network_id,
                "address network mismatch"
            );
            anyhow::ensure!(
                outputs > 0 && outputs <= 100 && ada > 0,
                "provide 1..100 positive ADA outputs"
            );
            let amount = ada.checked_mul(1_000_000).context("ADA amount overflow")?;
            let total = amount
                .checked_mul(u64::from(outputs))
                .context("output total overflow")?;
            let candidates: Vec<_> = koios
                .address_utxos(&from)
                .await?
                .iter()
                .filter(|u| !u.has_native_assets())
                .map(coin_select::WalletUtxo::try_from)
                .collect::<Result<Vec<_>>>()?;
            let tip = koios.tip().await?;
            let protocol = koios.protocol_params().await?;
            let cost = tx::fees::utxo_cost(&protocol)?;
            anyhow::ensure!(
                amount >= tx::fees::minimum_value_coin(&to, cost, &[], None)?,
                "requested transfer output is below minimum ADA"
            );
            let reserve = tx::fees::maximum_fee(&protocol, 0, false)?;
            let (selected, input_total) =
                coin_select::select_with_change(&candidates, &from, cost, reserve, total)?;
            let unsigned = tx::fees::balance(
                &protocol,
                input_total
                    .checked_sub(total)
                    .context("insufficient transfer funds")?,
                0,
                1,
                |fee, change| {
                    let mut staging = StagingTransaction::new()
                        .fee(fee)
                        .valid_from_slot(tip.abs_slot.saturating_sub(120))
                        .invalid_from_slot(tip.abs_slot + 7200)
                        .network_id(network_id)
                        .disclosed_signer(Hash::from(<[u8; 28]>::try_from(&source_bytes[1..29])?));
                    for _ in 0..outputs {
                        staging = staging.output(Output::new(destination.clone(), amount));
                    }
                    staging = staging.output(Output::new(source.clone(), change));
                    for input in &selected {
                        staging = staging.input(Input::new(
                            Hash::from(<[u8; 32]>::try_from(
                                hex::decode(&input.tx_hash)?.as_slice(),
                            )?),
                            input.output_index,
                        ));
                    }
                    let built = staging
                        .build_conway_raw()
                        .map_err(|e| anyhow::anyhow!("build wallet transfer: {e}"))?;
                    Ok(tx::UnsignedTx::new(
                        "ADA wallet transfer",
                        hex::encode(built.tx_bytes.0),
                    ))
                },
            )?;
            minos::preflight::transaction(
                koios,
                &unsigned.cbor_hex,
                &protocol,
                if network_id == 0 {
                    1_666_656_000
                } else {
                    1_591_566_291
                },
                &[],
            )
            .await?;
            anyhow::ensure!(!out.exists(), "output already exists");
            write_tx(unsigned, Some(&out))?;
        }
        WalletCmd::Fetch { hash, out } => {
            anyhow::ensure!(
                hex::decode(&hash)?.len() == 32,
                "transaction hash must be 32 bytes"
            );
            let cbor = match koios {
                DataSource::Koios(client) => client.transaction_cbor(&hash).await?,
                DataSource::Node(_) => {
                    anyhow::bail!("historical transaction fetch requires the Koios HTTP backend")
                }
            };
            let bytes = hex::decode(&cbor)?;
            let actual = hex::encode(tx::sign::tx_body_hash(&bytes)?);
            anyhow::ensure!(
                actual == hash.to_lowercase(),
                "provider returned a different transaction body"
            );
            minos::airgap::write_new(
                &out,
                &serde_json::json!({"type":"Tx ConwayEra",
                "description":"Confirmed transaction fetched from Koios", "cborHex":cbor}),
            )?;
            println!("Fetched and verified {actual} to {}", out.display());
        }
        WalletCmd::Inspect { tx: path } => {
            let bytes = hex::decode(read_cbor_hex(&path)?)?;
            let decoded: pallas_primitives::conway::Tx<'_> =
                pallas_codec::minicbor::decode(&bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "txId":hex::encode(tx::sign::tx_body_hash(&bytes)?),
                    "bytes":bytes.len(),
                    "outputAddresses":decoded.transaction_body.outputs.iter().map(|o| -> Result<String> {
                        Ok(pallas_traverse::MultiEraOutput::from_conway(o).address()?.to_bech32()?)
                    }).collect::<Result<Vec<_>>>()?,
                    "body":decoded.transaction_body,
                    "metadata":decoded.auxiliary_data,
                    "vkeyWitnesses":decoded.transaction_witness_set.vkeywitness
                }))?
            );
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
            let (vkey, signature) = wallet::sign_tx_body(&mnemonic, account, &body_hash)?;

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
