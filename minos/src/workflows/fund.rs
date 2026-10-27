//! fund command workflow, independent of CLI argument parsing.
use super::*;

// ---------------------------------------------------------------------------
// Milestone JSON schema (for --milestones file in fund command)
// ---------------------------------------------------------------------------

type MilestoneJson = minos::project_template::Milestone;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_fund(
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
    deposit: bool,
    reuse_treasury: Option<&str>,
    change_addr: Option<&str>,
    vendor_key_hex: Option<&str>,
    co_signer_hex: &[String],
) -> Result<()> {
    let normalized_policy = token_policy_hex.map(|value| value.trim().to_ascii_lowercase());
    let normalized_name = token_name_hex.map(|value| value.trim().to_ascii_lowercase());
    let token_policy_hex = normalized_policy.as_deref();
    let token_name_hex = normalized_name.as_deref();
    let vendor_key_hash = vendor_key_hex.map(hex::decode).transpose()?;
    let co_signer_hashes: Vec<Vec<u8>> = co_signer_hex
        .iter()
        .map(hex::decode)
        .collect::<std::result::Result<_, _>>()?;
    for key in co_signer_hashes.iter().chain(vendor_key_hash.iter()) {
        anyhow::ensure!(
            key.len() == 28,
            "vendor and co-signer keys must be 28 bytes"
        );
    }
    if let Some(change) = change_addr {
        let from = pallas_addresses::Address::from_bech32(fee_addr)?.to_vec();
        let to = pallas_addresses::Address::from_bech32(change)?.to_vec();
        anyhow::ensure!(from.len()>=29 && to.len()>=29 && from[0]&15==to[0]&15
            && from[1..29]==to[1..29] && to[0]>>4==0,
            "change address must be a base key address with the same payment credential and network");
    }
    // Parse inputs
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(milestones_path)?)?;
    let milestone_specs: Vec<MilestoneJson> = serde_json::from_value(if raw.is_array() {
        raw
    } else {
        raw["milestones"].clone()
    })
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

    let total_token_needed = milestone_specs.iter().try_fold(0u64, |total, m| {
        total
            .checked_add(m.token_amount)
            .context("milestone token total overflow")
    })?;
    let total_lovelace_needed = milestone_specs.iter().try_fold(0u64, |total, m| {
        total
            .checked_add(m.lovelace)
            .context("milestone ADA total overflow")
    })?;
    anyhow::ensure!(
        reuse_treasury.is_none() || !deposit || total_token_needed == 0,
        "treasury deposit reuse supports ADA-only initialization"
    );
    let spends_treasury = !deposit || reuse_treasury.is_some();

    // Fetch portal instance for vendor script hash
    let milestones: Vec<tx::fund::MilestoneSpec> = milestone_specs
        .into_iter()
        .map(|m| tx::fund::MilestoneSpec {
            id: m.id,
            label: m.label,
            description: m.description,
            acceptance_criteria: m.acceptance_criteria,
            requires_evidence: m.requires_evidence,
            maturation_ms: m.maturation_ms,
            lovelace: m.lovelace,
            token_amount: m.token_amount,
        })
        .collect();

    let inst = portal.instance_by_id(instance).await?;
    let proto = koios.protocol_params().await?;
    let cost = tx::fees::utxo_cost(&proto)?;
    let vendor_script_hash =
        hex::decode(&inst.registry.vendor_script.script_hash).context("vendor script hash hex")?;
    anyhow::ensure!(
        vendor_script_hash.len() == 28,
        "vendor script hash must be 28 bytes"
    );

    // Fetch wallet UTxOs from Koios
    let wallet_utxos = koios.address_utxos(fee_addr).await?;
    anyhow::ensure!(!wallet_utxos.is_empty(), "no UTxOs at {fee_addr}");
    let checked_wallet = wallet_utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;
    coin_select::select_inputs(&checked_wallet, 0)?;
    let treasury_hash: [u8; 28] = hex::decode(&inst.registry.treasury_script.script_hash)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid treasury hash"))?;
    let treasury_address = minos::scripts::base_script_addr(
        &pallas_crypto::hash::Hash::from(treasury_hash),
        network_id,
    )
    .to_bech32()?;
    let planned_datum = tx::fund::project_datum(
        vendor_key_hash.as_deref().unwrap_or(&admin_key_hash),
        &milestones,
        token_policy.as_deref(),
        token_name.as_deref(),
    )?;
    let output_floor = total_lovelace_needed.max(tx::fund::minimum_output_coin_for(
        if deposit {
            &treasury_hash
        } else {
            &vendor_script_hash
        },
        network_id,
        cost,
        &planned_datum,
        deposit,
    )?);
    let mut cached_references = Vec::new();
    let treasury_refs = if spends_treasury {
        let registry = references::registry_output(
            koios,
            instance,
            inst.registry
                .registry_reference
                .as_ref()
                .map(|reference| (reference.transaction.clone(), reference.index)),
        )
        .await?;
        let registry_ref = (registry.tx_hash, registry.tx_index);
        let reference = inst
            .registry
            .treasury_script
            .script_reference
            .as_ref()
            .context("treasury reference script is not confirmed")?;
        let treasury_ref = (reference.transaction.clone(), reference.index);
        cached_references = references::funding_references(
            koios,
            instance,
            registry_ref.clone(),
            treasury_ref.clone(),
        )
        .await?;
        Some(vec![registry_ref, treasury_ref])
    } else {
        None
    };
    let reference_bytes = references::total_script_bytes(&cached_references)?;
    let fee_reserve = tx::fees::maximum_fee(&proto, reference_bytes, spends_treasury)?;
    let change_address = pallas_addresses::Address::from_bech32(change_addr.unwrap_or(fee_addr))?;
    let minimum_change = tx::fees::minimum_builder_coin(cost, |coin| {
        Ok(pallas_txbuilder::Output::new(change_address.clone(), coin))
    })?;
    let mut utxos = if !spends_treasury {
        wallet_utxos.clone()
    } else {
        koios
            .address_utxos(&treasury_address)
            .await?
            .into_iter()
            .filter(|u| u.reference_script.is_none())
            .collect()
    };
    if let Some(reference) = reuse_treasury {
        let (hash, index) = parse_utxo_ref(reference)?;
        utxos.retain(|u| u.tx_hash == hash && u.tx_index == index);
        anyhow::ensure!(
            utxos.len() == 1,
            "requested treasury deposit is not unspent at this budget"
        );
        if deposit {
            anyhow::ensure!(
                utxos[0].lovelace() == output_floor && !utxos[0].has_native_assets(),
                "reused deposit must match the exact ADA amount"
            );
        }
    }

    // Reject malformed values and duplicate outputs before selecting any funds.
    let validated = utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;
    coin_select::select_inputs(&validated, 0)?;

    // Select inputs: prefer UTxOs with the right tokens, then fill with ADA-only inputs.
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
                total_input_tokens = total_input_tokens
                    .checked_add(qty)
                    .context("input token total overflow")?;
                total_input_lovelace = total_input_lovelace
                    .checked_add(u.lovelace())
                    .context("input ADA total overflow")?;
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
    } else if spends_treasury {
        let selected = tx::fund::select_ada_treasury_input(&utxos, total_lovelace_needed)?;
        total_input_lovelace = selected.lovelace();
        inputs.push(tx::fund::FundInput {
            tx_hash: selected.tx_hash.clone(),
            output_index: selected.tx_index,
            lovelace: selected.lovelace(),
            token_amount: 0,
        });
    }

    // Second pass: collect additional ADA UTxOs if needed.
    let vendor_floor = output_floor;
    let treasury_input = if !spends_treasury {
        None
    } else {
        anyhow::ensure!(inputs.len()==1, "funding needs one treasury UTxO holding the template's assets; use deposit-project first");
        Some((inputs[0].tx_hash.clone(), inputs[0].output_index))
    };
    let treasury_return_lovelace = if deposit {
        0
    } else {
        total_input_lovelace
            .checked_sub(total_lovelace_needed)
            .context("treasury has insufficient ADA")?
    };
    let target_lovelace = vendor_floor
        .checked_add(treasury_return_lovelace)
        .and_then(|coin| coin.checked_add(fee_reserve))
        .and_then(|coin| coin.checked_add(minimum_change))
        .context("funding target overflow")?;
    if spends_treasury {
        utxos.extend(
            wallet_utxos
                .iter()
                .filter(|u| !u.has_native_assets())
                .cloned(),
        );
    }
    let already_used: std::collections::HashSet<(String, u64)> = inputs
        .iter()
        .map(|i| (i.tx_hash.clone(), i.output_index))
        .collect();
    let fee_candidates: Vec<_> = if !spends_treasury {
        utxos.iter().filter(|u| !u.has_native_assets()).collect()
    } else {
        wallet_utxos
            .iter()
            .filter(|u| !u.has_native_assets())
            .collect()
    };
    for u in fee_candidates {
        if total_input_lovelace >= target_lovelace {
            break;
        }
        if already_used.contains(&(u.tx_hash.clone(), u.tx_index)) {
            continue;
        }
        total_input_lovelace = total_input_lovelace
            .checked_add(u.lovelace())
            .context("input ADA total overflow")?;
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

    let vendor_lovelace_total = output_floor;

    let change_lovelace = total_input_lovelace
        .saturating_sub(vendor_lovelace_total)
        .saturating_sub(treasury_return_lovelace)
        .checked_sub(fee_reserve)
        .context("insufficient ADA for funding fee reserve")?;
    let token_remainder = total_input_tokens.saturating_sub(total_token_needed);
    let change_tokens = if deposit { token_remainder } else { 0 };

    // Collect extra (non-target) native assets from selected inputs so they go back in change.
    let selected_outputs: std::collections::HashSet<(String, u64)> = inputs
        .iter()
        .map(|i| (i.tx_hash.clone(), i.output_index))
        .collect();
    let mut extra_assets = std::collections::BTreeMap::<(Vec<u8>, Vec<u8>), u64>::new();
    for u in &utxos {
        if !selected_outputs.contains(&(u.tx_hash.clone(), u.tx_index)) {
            continue;
        }
        if let Some(assets) = &u.asset_list {
            for a in assets {
                let is_target = token_policy_hex.map(|p| p == a.policy_id).unwrap_or(false)
                    && token_name_hex.map(|n| n == a.asset_name).unwrap_or(false);
                if is_target {
                    continue;
                }
                let qty: u64 = a.quantity.parse().context("invalid change quantity")?;
                let policy = hex::decode(&a.policy_id).context("invalid change policy")?;
                let name = hex::decode(&a.asset_name).context("invalid change asset name")?;
                let total = extra_assets.entry((policy, name)).or_default();
                *total = total.checked_add(qty).context("change asset overflow")?;
            }
        }
    }

    let mut extra_change_assets = extra_assets
        .into_iter()
        .map(|((policy, name), qty)| (policy, name, qty))
        .collect::<Vec<_>>();

    let tip = koios.tip().await?;
    let valid_from_slot = tip.abs_slot.saturating_sub(120);
    let invalid_from_slot = tip.abs_slot + 7200;

    let mut treasury = None;
    if let Some(input) = treasury_input {
        let collateral = tx::fund::select_funding_collateral(&checked_wallet, &inputs, 0)?.clone();
        let mut return_assets = std::mem::take(&mut extra_change_assets);
        if token_remainder > 0 {
            return_assets.push((
                token_policy.clone().context("missing token policy")?,
                token_name.clone().context("missing token name")?,
                token_remainder,
            ));
        }
        treasury = Some(tx::fund::TreasurySpend {
            input,
            references: treasury_refs
                .clone()
                .context("missing treasury reference plan")?,
            collateral,
            cost_model: proto.v3_cost_model().context("missing cost model")?,
            return_address: treasury_address,
            return_lovelace: treasury_return_lovelace,
            return_assets,
        });
    }
    let mut params = tx::fund::FundParams {
        treasury,
        instance_id: instance.to_string(),
        vendor_script_hash,
        admin_key_hash,
        vendor_key_hash,
        co_signer_hashes,
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
        fee_addr: change_addr.unwrap_or(fee_addr).to_string(),
        fee: fee_reserve,
        coins_per_utxo_byte: cost,
        valid_from_slot,
        invalid_from_slot,
        network_id,
    };

    let mut signer_keys = std::collections::BTreeSet::from([params.admin_key_hash.clone()]);
    signer_keys.extend(params.co_signer_hashes.iter().cloned());
    if !deposit {
        signer_keys.extend(params.vendor_key_hash.iter().cloned());
    }
    let source_address = pallas_addresses::Address::from_bech32(fee_addr)?.to_vec();
    anyhow::ensure!(
        source_address.len() >= 29
            && matches!(source_address[0] >> 4, 0 | 2 | 4 | 6)
            && source_address[0] & 15 == network_id,
        "fee address must be a payment key on the selected network"
    );
    signer_keys.insert(source_address[1..29].to_vec());
    let available = total_input_lovelace
        .checked_sub(vendor_lovelace_total)
        .and_then(|coin| coin.checked_sub(treasury_return_lovelace))
        .context("funding outputs exceed selected ADA")?;
    let unsigned_tx = tx::fees::balance(
        &proto,
        available,
        reference_bytes,
        signer_keys.len() as u64,
        |fee, change| {
            params.fee = fee;
            params.change_lovelace = change;
            if let Some(treasury) = &mut params.treasury {
                let collateral = tx::fund::select_funding_collateral(
                    &checked_wallet,
                    &params.inputs,
                    tx::fees::minimum_collateral(fee, &proto)?,
                )?
                .clone();
                treasury.collateral = collateral;
            }
            if deposit {
                tx::fund::build_deposit(params.clone(), &treasury_hash)
            } else {
                tx::fund::build(params.clone())
            }
        },
    )?;
    if let Some(treasury) = &params.treasury {
        tx::fees::validate_collateral(&unsigned_tx.cbor_hex, &proto, &treasury.collateral)?;
    }
    let report = minos::preflight::transaction(
        koios,
        &unsigned_tx.cbor_hex,
        &proto,
        if network_id == 0 {
            1_666_656_000
        } else {
            1_591_566_291
        },
        &cached_references,
    )
    .await?;
    eprintln!(
        "Funding preflight passed: {} redeemers, {} memory, {} steps",
        report.redeemers, report.memory, report.steps
    );
    write_tx(unsigned_tx, out)
}
