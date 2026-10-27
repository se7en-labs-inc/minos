use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_codec::minicbor;
use pallas_codec::utils::{Int, KeyValuePairs, MaybeIndefArray};
use pallas_crypto::hash::{Hash, Hasher};
use pallas_primitives::conway::{BigInt, BoundedBytes, Constr, PlutusData};
use pallas_txbuilder::{BuildConway, Input, Output, ScriptKind, StagingTransaction};

use super::UnsignedTx;

// CIP-68 asset name prefixes
const FT_PREFIX: &[u8] = &[0x00, 0x14, 0xdf, 0x10];
const REF_NFT_PREFIX: &[u8] = &[0x00, 0x06, 0x43, 0xb0];

#[derive(Clone)]
pub struct MintInput {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
}

#[derive(Clone)]
pub struct MintParams {
    // Policy key — the native script is a single-sig on this key hash.
    pub admin_key_hash: Vec<u8>, // 28 bytes

    // How many FTs to mint (base units — multiply by 10^decimals for "whole" tokens).
    pub ft_amount: u64,
    pub mint_reference: bool,

    // Token info
    pub token_ticker: String, // e.g. "USDM7"
    pub token_decimals: u64,  // e.g. 6

    // Inputs: provide ADA for outputs + fee.
    pub inputs: Vec<MintInput>,

    // Destination for minted FTs. Reference NFT also goes here (wallet keeps it).
    pub dest_addr: String,
    pub fee_addr: String,

    pub fee: u64,
    pub coins_per_utxo_byte: u64,
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,
    pub network_id: u8,
}

pub struct MintResult {
    pub unsigned_tx: UnsignedTx,
    pub policy_id_hex: String,
    pub ft_asset_name_hex: String,
    pub ref_nft_asset_name_hex: String,
}

fn token_output(params: &MintParams, coin: u64) -> Result<Output> {
    anyhow::ensure!(
        params.ft_amount > 0 && params.ft_amount <= i64::MAX as u64,
        "mint amount must fit a positive signed 64-bit integer"
    );
    anyhow::ensure!(
        params.token_ticker.len() <= 28,
        "ticker exceeds asset name limit"
    );
    let admin: [u8; 28] = params
        .admin_key_hash
        .as_slice()
        .try_into()
        .context("admin key must be 28 bytes")?;
    let policy = native_policy_id(&native_sig_script_cbor(&admin));
    let name = [FT_PREFIX, params.token_ticker.as_bytes()].concat();
    let mut output = Output::new(PallasAddress::from_bech32(&params.dest_addr)?, coin).add_asset(
        policy,
        name,
        params.ft_amount,
    )?;
    if params.mint_reference {
        let name = [REF_NFT_PREFIX, params.token_ticker.as_bytes()].concat();
        let description = format!("{} ({}dp)", params.token_ticker, params.token_decimals);
        output = output
            .add_asset(policy, name, 1)?
            .set_inline_datum(cip68_datum_bytes(
                &params.token_ticker,
                &description,
                &params.token_ticker,
                params.token_decimals,
            ));
    }
    Ok(output)
}

pub fn minimum_output_coin(params: &MintParams) -> Result<u64> {
    super::fees::minimum_builder_coin(params.coins_per_utxo_byte, |coin| {
        token_output(params, coin)
    })
}

pub fn build(params: MintParams) -> Result<MintResult> {
    anyhow::ensure!(
        params.ft_amount > 0 && params.ft_amount <= i64::MAX as u64,
        "mint amount must fit a positive signed 64-bit integer"
    );
    anyhow::ensure!(
        params.token_ticker.len() <= 28,
        "ticker exceeds asset name limit"
    );
    // -----------------------------------------------------------------------
    // 1. Native script: [0, key_hash]
    // -----------------------------------------------------------------------
    let admin_arr: [u8; 28] = params
        .admin_key_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("admin_key_hash must be 28 bytes"))?;

    let script_cbor = native_sig_script_cbor(&admin_arr);
    let policy_hash = native_policy_id(&script_cbor);
    let policy_id_hex = hex::encode(*policy_hash);

    // -----------------------------------------------------------------------
    // 2. Asset names
    // -----------------------------------------------------------------------
    let ticker_bytes = params.token_ticker.as_bytes();
    let mut ft_name = FT_PREFIX.to_vec();
    ft_name.extend_from_slice(ticker_bytes);
    let mut ref_name = REF_NFT_PREFIX.to_vec();
    ref_name.extend_from_slice(ticker_bytes);

    let ft_asset_name_hex = hex::encode(&ft_name);
    let ref_nft_asset_name_hex = hex::encode(&ref_name);

    // -----------------------------------------------------------------------
    // 3. CIP-68 datum for reference NFT
    // -----------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // 4. Addresses
    // -----------------------------------------------------------------------
    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("fee addr bech32")?
        .to_vec();
    let fee_addr = PallasAddress::from_bytes(&fee_addr_bytes).context("fee addr bytes")?;

    // -----------------------------------------------------------------------
    // 5. Outputs
    //    (a) Reference NFT → dest_addr with CIP-68 datum
    //    (b) FTs → dest_addr (2 ADA min)
    //    (c) Change → fee_addr
    // -----------------------------------------------------------------------
    // Put both NFT and FTs in a single output to keep it simple.
    let output_coin = minimum_output_coin(&params)?;
    let token_output = token_output(&params, output_coin)?;
    let input_coin = params.inputs.iter().try_fold(0u64, |total, input| {
        total
            .checked_add(input.lovelace)
            .context("mint input ADA overflow")
    })?;
    let change_coin = input_coin
        .checked_sub(output_coin)
        .and_then(|coin| coin.checked_sub(params.fee))
        .context("insufficient ADA for mint output and fee")?;
    let change_output = Output::new(fee_addr, change_coin);

    // -----------------------------------------------------------------------
    // 6. Disclosed signer (policy key must sign)
    // -----------------------------------------------------------------------
    let admin_hash: Hash<28> = Hash::from(admin_arr);

    // -----------------------------------------------------------------------
    // 7. Build staging transaction
    // -----------------------------------------------------------------------
    let mut staging = StagingTransaction::new()
        .output(token_output)
        .output(change_output)
        .fee(params.fee)
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        .network_id(params.network_id)
        .disclosed_signer(admin_hash)
        // Native script in witness set
        .script(ScriptKind::Native, script_cbor)
        // Mint ref NFT + FTs
        .mint_asset(policy_hash, ft_name.clone(), params.ft_amount as i64)
        .context("mint FTs")?;

    if params.mint_reference {
        staging = staging
            .mint_asset(policy_hash, ref_name.clone(), 1)
            .context("mint ref NFT")?;
    }

    let mut seen = std::collections::BTreeSet::new();
    for inp in &params.inputs {
        let h: [u8; 32] = hex::decode(&inp.tx_hash)
            .context("mint input hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("mint input hash must be 32 bytes"))?;
        anyhow::ensure!(seen.insert((h, inp.output_index)), "duplicate mint input");
        staging = staging.input(Input::new(Hash::from(h), inp.output_index));
    }

    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    let cbor_hex = hex::encode(&built.tx_bytes.0);

    Ok(MintResult {
        unsigned_tx: UnsignedTx::new(
            format!("mint {} {}", params.ft_amount, params.token_ticker),
            cbor_hex,
        ),
        policy_id_hex,
        ft_asset_name_hex,
        ref_nft_asset_name_hex,
    })
}

// ---------------------------------------------------------------------------
// Native script helpers
// ---------------------------------------------------------------------------

/// CBOR: array(2) [0, bytes(28)]
fn native_sig_script_cbor(key_hash: &[u8; 28]) -> Vec<u8> {
    let mut cbor = Vec::with_capacity(31);
    cbor.push(0x82); // array(2)
    cbor.push(0x00); // uint(0) = sig type
    cbor.push(0x58); // bytes, 1-byte length
    cbor.push(0x1c); // 28
    cbor.extend_from_slice(key_hash);
    cbor
}

/// blake2b-224(0x00 || script_cbor) — 0x00 is the native script tag byte.
fn native_policy_id(script_cbor: &[u8]) -> Hash<28> {
    let mut tagged = Vec::with_capacity(1 + script_cbor.len());
    tagged.push(0x00);
    tagged.extend_from_slice(script_cbor);
    Hasher::<224>::hash(&tagged)
}

// ---------------------------------------------------------------------------
// CIP-68 datum
// ---------------------------------------------------------------------------

/// Encodes the CIP-68 V1 reference datum:
/// Constr 0 [Map {fields}, Int 1, Map {}]
fn cip68_datum_bytes(name: &str, description: &str, ticker: &str, decimals: u64) -> Vec<u8> {
    let mk_bstr = |s: &[u8]| PlutusData::BoundedBytes(BoundedBytes::from(s.to_vec()));
    let mk_int = |n: i128| PlutusData::BigInt(BigInt::Int(Int::try_from(n).expect("fits")));

    let fields_map = PlutusData::Map(KeyValuePairs::Def(vec![
        (mk_bstr(b"name"), mk_bstr(name.as_bytes())),
        (mk_bstr(b"description"), mk_bstr(description.as_bytes())),
        (mk_bstr(b"ticker"), mk_bstr(ticker.as_bytes())),
        (mk_bstr(b"decimals"), mk_int(decimals as i128)),
    ]));

    let datum = PlutusData::Constr(Constr {
        tag: 121, // constructor 0
        any_constructor: None,
        fields: MaybeIndefArray::Def(vec![
            fields_map,
            mk_int(1),                                   // version
            PlutusData::Map(KeyValuePairs::Def(vec![])), // extra
        ]),
    });

    minicbor::to_vec(datum).expect("CIP-68 datum encode")
}
