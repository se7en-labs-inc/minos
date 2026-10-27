//! Checked Conway fee and minimum-output calculations. See the pinned ledger
//! research in specs/001-open-source-release/research.md before extending eras.
use crate::koios::ProtocolParams;
use anyhow::{ensure, Context, Result};
use pallas_codec::minicbor;
use pallas_primitives::conway::{Redeemers, TransactionOutput, Tx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rational {
    numerator: u128,
    denominator: u128,
}

impl Rational {
    fn new(numerator: u128, denominator: u128) -> Result<Self> {
        ensure!(denominator > 0, "zero price denominator");
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let divisor = a.max(1);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
    fn add(self, other: Self) -> Result<Self> {
        let numerator = self
            .numerator
            .checked_mul(other.denominator)
            .and_then(|n| {
                other
                    .numerator
                    .checked_mul(self.denominator)
                    .and_then(|m| n.checked_add(m))
            })
            .context("rational price overflow")?;
        Self::new(
            numerator,
            self.denominator
                .checked_mul(other.denominator)
                .context("price denominator overflow")?,
        )
    }
    fn multiply(self, numerator: u128, denominator: u128) -> Result<Self> {
        Self::new(
            self.numerator
                .checked_mul(numerator)
                .context("price overflow")?,
            self.denominator
                .checked_mul(denominator)
                .context("price denominator overflow")?,
        )
    }
    fn floor(self) -> Result<u64> {
        u64::try_from(self.numerator / self.denominator).context("fee exceeds ledger coin range")
    }
    fn ceil(self) -> Result<u64> {
        u64::try_from(self.numerator.div_ceil(self.denominator))
            .context("fee exceeds ledger coin range")
    }
}

fn price(params: &ProtocolParams, name: &str) -> Result<Rational> {
    let exact = params.parameter(&format!("exact_{name}"));
    if !exact.is_null() {
        return Rational::new(
            exact["numerator"]
                .as_u64()
                .context("invalid price numerator")?
                .into(),
            exact["denominator"]
                .as_u64()
                .context("invalid price denominator")?
                .into(),
        );
    }
    let value = params.parameter(name);
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent
                .parse::<i32>()
                .context("invalid decimal price exponent")?,
        ),
        None => (text.as_str(), 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    ensure!(
        whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
            && !whole.is_empty(),
        "missing or invalid price {name}"
    );
    let digits = format!("{whole}{fraction}");
    let numerator: u128 = digits.parse().context("invalid decimal price")?;
    let scale = i32::try_from(fraction.len())?
        .checked_sub(exponent)
        .context("decimal price scale overflow")?;
    ensure!(
        (-18..=18).contains(&scale),
        "unsupported decimal price precision"
    );
    if scale >= 0 {
        Rational::new(numerator, 10u128.pow(scale as u32))
    } else {
        Rational::new(
            numerator
                .checked_mul(10u128.pow((-scale) as u32))
                .context("price overflow")?,
            1,
        )
    }
}

fn integer(params: &ProtocolParams, name: &str) -> Result<u64> {
    let value = params.parameter(name);
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
        .with_context(|| format!("missing or invalid protocol parameter {name}"))
}

pub fn reference_fee(params: &ProtocolParams, mut size: u64) -> Result<u64> {
    ensure!(
        size <= 200 * 1024,
        "reference scripts exceed Conway transaction limit"
    );
    let mut cost = price(params, "min_fee_ref_script_cost_per_byte")?;
    let mut total = Rational::new(0, 1)?;
    while size > 0 {
        let count = size.min(25_600);
        total = total.add(cost.multiply(count.into(), 1)?)?;
        size -= count;
        cost = cost.multiply(6, 5)?;
    }
    total.floor()
}

/// Selection reserve bounded by the supported protocol's transaction limits.
/// The final fee is still measured from the rebuilt transaction.
pub fn maximum_fee(params: &ProtocolParams, reference_bytes: u64, plutus: bool) -> Result<u64> {
    let (_, memory, steps) = params.evaluation_limits()?;
    let linear = params
        .min_fee_a
        .checked_mul(params.max_tx_size)
        .and_then(|fee| fee.checked_add(params.min_fee_b))
        .context("fee reserve overflow")?;
    let execution = if plutus {
        price(params, "price_mem")?
            .multiply(memory.into(), 1)?
            .add(price(params, "price_step")?.multiply(steps.into(), 1)?)?
            .ceil()?
    } else {
        0
    };
    let reference = reference_fee(params, reference_bytes)?;
    linear
        .checked_add(execution)
        .and_then(|fee| fee.checked_add(reference))
        .context("fee reserve overflow")
}

/// Calculate size by inserting synthetic witnesses into an in-memory copy only.
/// This measures map/set headers at CBOR width boundaries without loading keys.
pub fn signed_size(bytes: &[u8], witnesses: u64) -> Result<u64> {
    let tx: Tx<'_> = minicbor::decode(bytes)?;
    ensure!(
        tx.transaction_witness_set
            .vkeywitness
            .as_ref()
            .is_none_or(|w| w.is_empty()),
        "fee balancing requires an unsigned transaction"
    );
    ensure!(witnesses <= 1024, "too many transaction witnesses");
    let mut sized = bytes.to_vec();
    for index in 0..witnesses {
        let mut key = [0; 32];
        key[..8].copy_from_slice(&index.to_be_bytes());
        sized = super::sign::add_vkey_witness(&sized, &key, &[0; 64])?;
    }
    Ok(sized.len() as u64)
}

pub fn witness_count<'a>(
    keys: impl IntoIterator<Item = &'a [u8]>,
    fee_address: &str,
    network: u8,
) -> Result<u64> {
    let mut unique = std::collections::BTreeSet::new();
    for key in keys {
        let key: [u8; 28] = key.try_into().context("signer key hash must be 28 bytes")?;
        unique.insert(key);
    }
    unique.insert(crate::coin_select::address_key_hash(fee_address, network)?);
    Ok(unique.len() as u64)
}

pub fn minimum_fee(
    cbor_hex: &str,
    params: &ProtocolParams,
    reference_bytes: u64,
    witnesses: u64,
) -> Result<u64> {
    let (major, max_mem, max_steps) = params.evaluation_limits()?;
    ensure!((9..=11).contains(&major), "unsupported Conway fee protocol");
    let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
    let tx: Tx<'_> = minicbor::decode(&bytes)?;
    let (mut memory, mut steps) = (0u64, 0u64);
    let units = match tx.transaction_witness_set.redeemer.as_deref() {
        Some(Redeemers::List(items)) => items.iter().map(|r| r.ex_units).collect::<Vec<_>>(),
        Some(Redeemers::Map(items)) => items.values().map(|r| r.ex_units).collect(),
        None => vec![],
    };
    for units in units {
        memory = memory
            .checked_add(units.mem)
            .context("execution memory overflow")?;
        steps = steps
            .checked_add(units.steps)
            .context("execution steps overflow")?;
    }
    ensure!(
        memory <= max_mem && steps <= max_steps,
        "declared execution units exceed protocol limits"
    );
    let size = signed_size(&bytes, witnesses)?;
    ensure!(
        size <= params.max_tx_size,
        "signed transaction exceeds maximum size: {size} bytes > {} bytes",
        params.max_tx_size
    );
    let linear = params
        .min_fee_a
        .checked_mul(size)
        .and_then(|n| n.checked_add(params.min_fee_b))
        .context("linear fee overflow")?;
    let execution = if memory == 0 && steps == 0 {
        0
    } else {
        price(params, "price_mem")?
            .multiply(memory.into(), 1)?
            .add(price(params, "price_step")?.multiply(steps.into(), 1)?)?
            .ceil()?
    };
    let reference = reference_fee(params, reference_bytes)?;
    linear
        .checked_add(execution)
        .and_then(|n| n.checked_add(reference))
        .context("total fee overflow")
}

pub fn minimum_output(output: &TransactionOutput<'_>, params: &ProtocolParams) -> Result<u64> {
    minimum_output_at_cost(output, utxo_cost(params)?)
}

pub fn utxo_cost(params: &ProtocolParams) -> Result<u64> {
    let cost = integer(params, "coins_per_utxo_size")?;
    ensure!(cost > 0, "coins per UTxO byte must be positive");
    Ok(cost)
}

fn minimum_output_at_cost(output: &TransactionOutput<'_>, cost: u64) -> Result<u64> {
    ensure!(cost > 0, "coins per UTxO byte must be positive");
    (minicbor::to_vec(output)?.len() as u64)
        .checked_add(160)
        .and_then(|size| size.checked_mul(cost))
        .context("minimum output coin overflow")
}

/// Measure the actual builder output, including datum/assets/reference script,
/// and iterate until the coin's own CBOR width is reflected in the minimum.
pub fn minimum_builder_coin(
    cost: u64,
    mut output: impl FnMut(u64) -> Result<pallas_txbuilder::Output>,
) -> Result<u64> {
    use pallas_txbuilder::{BuildConway, Input, StagingTransaction};
    let mut coin = 0;
    for _ in 0..8 {
        let built = StagingTransaction::new()
            .input(Input::new([0; 32].into(), 0))
            .output(output(coin)?)
            .fee(0)
            .valid_from_slot(0)
            .invalid_from_slot(1)
            .network_id(0)
            .build_conway_raw()
            .map_err(|error| anyhow::anyhow!("measure output: {error}"))?;
        let tx: Tx<'_> = minicbor::decode(&built.tx_bytes.0)?;
        let required = minimum_output_at_cost(&tx.transaction_body.outputs[0], cost)?;
        if coin >= required {
            return Ok(coin);
        }
        coin = required;
    }
    anyhow::bail!("minimum output coin did not converge")
}

/// Supply any minimum-ADA shortfall from wallet change, preserving the value
/// already held by the contract and all native assets.
pub fn funded_output(
    cost: u64,
    requested: u64,
    wallet_change: &mut u64,
    mut output: impl FnMut(u64) -> Result<pallas_txbuilder::Output>,
) -> Result<pallas_txbuilder::Output> {
    let coin = requested.max(minimum_builder_coin(cost, &mut output)?);
    *wallet_change = wallet_change
        .checked_sub(coin - requested)
        .context("wallet change cannot cover output minimum ADA")?;
    output(coin)
}

pub fn minimum_value_coin(
    address: &str,
    cost: u64,
    assets: &[(String, u64)],
    datum: Option<&[u8]>,
) -> Result<u64> {
    let address = pallas_addresses::Address::from_bech32(address)?;
    minimum_builder_coin(cost, |coin| {
        let output = pallas_txbuilder::Output::new(address.clone(), coin);
        let output = if let Some(datum) = datum {
            output.set_inline_datum(datum.to_vec())
        } else {
            output
        };
        super::value::add_assets(output, assets)
    })
}

pub fn minimum_collateral(fee: u64, params: &ProtocolParams) -> Result<u64> {
    ensure!(
        params.max_collateral_inputs > 0 && params.collateral_percent > 0,
        "invalid collateral parameters"
    );
    u64::try_from((u128::from(fee) * u128::from(params.collateral_percent)).div_ceil(100))
        .context("collateral overflow")
}

/// Validate the one ADA-only collateral input supported by these builders.
/// An input consumed for normal balancing cannot also cover phase-two failure.
pub fn validate_collateral(
    cbor_hex: &str,
    params: &ProtocolParams,
    collateral: &crate::coin_select::WalletUtxo,
) -> Result<()> {
    ensure!(
        !collateral.has_native_assets && collateral.assets.is_empty(),
        "collateral must be ADA-only"
    );
    let bytes = hex::decode(cbor_hex)?;
    let tx: Tx<'_> = minicbor::decode(&bytes)?;
    let body = &tx.transaction_body;
    let inputs = body
        .collateral
        .as_ref()
        .context("script transaction has no collateral")?;
    ensure!(
        inputs.len() == 1 && inputs.len() as u64 <= params.max_collateral_inputs,
        "builder requires exactly one collateral input within protocol limits"
    );
    let input = &inputs[0];
    ensure!(
        input.transaction_id.to_string() == collateral.tx_hash.to_ascii_lowercase()
            && input.index == collateral.output_index,
        "collateral resolution mismatch"
    );
    ensure!(
        !body.inputs.iter().any(|spend| spend == input),
        "collateral is also a spending input"
    );
    ensure!(
        collateral.lovelace >= minimum_collateral(body.fee, params)?,
        "insufficient collateral for calculated fee and protocol percentage"
    );
    Ok(())
}

/// Rebuild fee and change until their encoded sizes stabilize. All other outputs
/// must already contain their contract-required value and protocol minimum coin.
pub fn balance(
    params: &ProtocolParams,
    available_coin: u64,
    reference_bytes: u64,
    witnesses: u64,
    mut build: impl FnMut(u64, u64) -> Result<super::UnsignedTx>,
) -> Result<super::UnsignedTx> {
    let mut fee = 0;
    for _ in 0..16 {
        let change = available_coin
            .checked_sub(fee)
            .context("insufficient ADA for fee")?;
        let built = build(fee, change)?;
        let required = minimum_fee(&built.cbor_hex, params, reference_bytes, witnesses)?;
        if fee >= required {
            let bytes = hex::decode(&built.cbor_hex)?;
            let tx: Tx<'_> = minicbor::decode(&bytes)?;
            for output in &tx.transaction_body.outputs {
                ensure!(
                    pallas_traverse::MultiEraOutput::from_conway(output)
                        .value()
                        .coin()
                        >= minimum_output(output, params)?,
                    "output below protocol minimum ADA; add wallet funds before rebuilding"
                );
            }
            return Ok(built);
        }
        fee = required;
    }
    anyhow::bail!("fee/change calculation did not converge")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};

    fn parameters() -> ProtocolParams {
        serde_json::from_value(serde_json::json!({
            "min_fee_a": 44, "min_fee_b": 155381, "max_tx_size": 16384,
            "collateral_percent": 150, "max_collateral_inputs": 3,
            "protocol_major": 10, "max_tx_ex_mem": 14000000, "max_tx_ex_steps": 10000000000u64,
            "coins_per_utxo_size": 4310, "price_mem": "0.0577", "price_step": "7.21e-5",
            "min_fee_ref_script_cost_per_byte": "15"
        }))
        .unwrap()
    }

    fn transaction(fee: u64, change: u64, datum_size: usize) -> super::super::UnsignedTx {
        let address = pallas_addresses::Address::from_bech32(
            "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh",
        )
        .unwrap();
        let mut output = Output::new(address, change);
        if datum_size > 0 {
            let datum =
                pallas_primitives::conway::PlutusData::BoundedBytes(vec![1; datum_size].into());
            output = output
                .set_inline_datum(minicbor::to_vec(datum).unwrap())
                .add_asset([2; 28].into(), vec![3; 32], 99)
                .unwrap();
        }
        let built = StagingTransaction::new()
            .input(Input::new([1; 32].into(), 0))
            .output(output)
            .fee(fee)
            .network_id(0)
            .valid_from_slot(1)
            .invalid_from_slot(2)
            .build_conway_raw()
            .unwrap();
        super::super::UnsignedTx::new("synthetic sizing fixture", hex::encode(built.tx_bytes.0))
    }

    #[test]
    fn reference_tiers_fractional_prices_and_limit_boundaries() {
        let p = parameters();
        for (size, expected) in [
            (0, 0),
            (25600, 384000),
            (25601, 384018),
            (51200, 844800),
            (51201, 844821),
            (76800, 1397760),
            (102400, 2061312),
            (128000, 2857574),
            (153600, 3813089),
            (179200, 4959707),
            // Independent geometric-series calculation for eight full tiers:
            // floor(384000 * (6^8 - 5^8) / 5^7).
            (204800, 494_972_544_000 / 78_125),
        ] {
            assert_eq!(reference_fee(&p, size).unwrap(), expected, "{size}");
        }
        assert!(reference_fee(&p, 204801).is_err());
        let p = p.with_ledger_fields(4310, (577, 10000), (721, 10000000), (31, 2));
        assert_eq!(reference_fee(&p, 51200).unwrap(), 872960);
        assert_eq!(
            price(&p, "price_mem").unwrap(),
            Rational::new(577, 10000).unwrap()
        );
        assert_eq!(
            price(&parameters(), "price_step").unwrap(),
            Rational::new(721, 10000000).unwrap()
        );
        assert!(reference_fee(&p.with_ledger_fields(4310, (1, 1), (1, 1), (1, 0)), 1).is_err());
    }

    #[test]
    fn witness_header_boundaries_and_size_limits_are_measured() {
        let tx = transaction(0, 5_000_000, 0);
        let bytes = hex::decode(&tx.cbor_hex).unwrap();
        assert_eq!(signed_size(&bytes, 0).unwrap(), bytes.len() as u64);
        assert_eq!(
            signed_size(&bytes, 24).unwrap() - signed_size(&bytes, 23).unwrap(),
            102
        );
        let mut p = parameters();
        p.max_tx_size = signed_size(&bytes, 24).unwrap();
        assert!(minimum_fee(&tx.cbor_hex, &p, 0, 24).is_ok());
        p.max_tx_size -= 1;
        assert!(minimum_fee(&tx.cbor_hex, &p, 0, 24).is_err());
        assert!(minimum_fee("zz", &p, 0, 0).is_err());
    }

    #[test]
    fn minimum_coin_accounts_for_large_datum_and_tokens() {
        let p = parameters();
        let mut minima = vec![];
        for size in [0, 4096] {
            let tx = transaction(0, 5_000_000, size);
            let bytes = hex::decode(tx.cbor_hex).unwrap();
            let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
            minima.push(minimum_output(&tx.transaction_body.outputs[0], &p).unwrap());
        }
        assert!(minima[0] < 1_000_000);
        assert!(minima[1] > 10_000_000);
        assert_eq!(minimum_collateral(1, &p).unwrap(), 2);
        assert_eq!(minimum_collateral(200_000, &p).unwrap(), 300_000);
    }

    #[test]
    fn collateral_tracks_fee_percentage_and_rejects_spent_or_token_inputs() {
        let params = parameters();
        let address = pallas_addresses::Address::from_bech32(
            "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh",
        )
        .unwrap();
        let build = |collateral_hash| {
            hex::encode(
                StagingTransaction::new()
                    .input(Input::new([1; 32].into(), 0))
                    .collateral_input(Input::new(collateral_hash, 0))
                    .output(Output::new(address.clone(), 2_000_000))
                    .fee(200_000)
                    .network_id(0)
                    .valid_from_slot(1)
                    .invalid_from_slot(2)
                    .build_conway_raw()
                    .unwrap()
                    .tx_bytes
                    .0,
            )
        };
        let mut collateral = crate::coin_select::WalletUtxo {
            tx_hash: "02".repeat(32),
            output_index: 0,
            lovelace: 300_000,
            has_native_assets: false,
            assets: Default::default(),
        };
        assert!(validate_collateral(&build([2; 32].into()), &params, &collateral).is_ok());
        collateral.lovelace = 299_999;
        assert!(validate_collateral(&build([2; 32].into()), &params, &collateral).is_err());
        collateral.lovelace = 300_000;
        collateral.has_native_assets = true;
        assert!(validate_collateral(&build([2; 32].into()), &params, &collateral).is_err());
        collateral.has_native_assets = false;
        collateral.tx_hash = "01".repeat(32);
        assert!(validate_collateral(&build([1; 32].into()), &params, &collateral).is_err());
    }

    #[test]
    fn balance_rebuilds_cbor_widths_and_rejects_insufficient_change() {
        let p = parameters();
        let mut builds = 0;
        let tx = balance(&p, 5_000_000, 25_601, 24, |fee, change| {
            builds += 1;
            Ok(transaction(fee, change, 0))
        })
        .unwrap();
        assert!(builds >= 2);
        let bytes = hex::decode(&tx.cbor_hex).unwrap();
        let decoded: Tx<'_> = minicbor::decode(&bytes).unwrap();
        assert!(decoded.transaction_body.fee >= minimum_fee(&tx.cbor_hex, &p, 25_601, 24).unwrap());
        assert_eq!(
            decoded.transaction_body.fee
                + pallas_traverse::MultiEraOutput::from_conway(
                    &decoded.transaction_body.outputs[0]
                )
                .value()
                .coin(),
            5_000_000
        );
        assert!(balance(&p, 1, 0, 1, |fee, change| Ok(transaction(fee, change, 0))).is_err());
        assert!(balance(&p, 5_000_000, 0, 1, |fee, change| Ok(transaction(
            fee, change, 4096
        )))
        .is_err());
    }
}
