/// BIP32-Ed25519 key derivation and signing for Cardano Shelley addresses.
///
/// Derivation path: m/1852'/1815'/<account>'/<role>/<index>
/// Root key: Icarus style — PBKDF2-HMAC-SHA512(password, salt=entropy, c=4096, dkLen=96)
use anyhow::{Context, Result};
use bip39::Mnemonic;
use cryptoxide::{hmac::Hmac, pbkdf2::pbkdf2, sha2::Sha512};
use ed25519_bip32::{DerivationScheme, XPrv};
use pallas_addresses::{
    Address, Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart,
};
use pallas_crypto::hash::{Hash, Hasher};

const SCHEME: DerivationScheme = DerivationScheme::V2;
const PURPOSE: u32 = 0x8000073C; // 1852'
const COIN_TYPE: u32 = 0x80000717; // 1815'

/// Derive the Icarus root XPrv from BIP39 entropy and an optional passphrase.
fn root_key(entropy: &[u8], passphrase: &[u8]) -> XPrv {
    let mut key = [0u8; 96];
    pbkdf2(
        &mut Hmac::new(Sha512::new(), passphrase),
        entropy,
        4096,
        &mut key,
    );
    XPrv::normalize_bytes_force3rd(key)
}

/// Derive the account XPrv at m/1852'/1815'/<account>'.
fn account_xprv(root: &XPrv, account: u32) -> XPrv {
    root.derive(SCHEME, PURPOSE)
        .derive(SCHEME, COIN_TYPE)
        .derive(SCHEME, account | 0x80000000)
}

/// Derive a child key at <role>/<index> (soft derivation) under an account key.
fn child_xprv(account: &XPrv, role: u32, index: u32) -> XPrv {
    account.derive(SCHEME, role).derive(SCHEME, index)
}

/// blake2b-224 of the 32-byte public key — the standard Cardano key hash.
fn vkey_hash(xprv: &XPrv) -> Hash<28> {
    let xpub = xprv.public();
    Hasher::<224>::hash(&xpub.as_ref()[..32])
}

/// Derive the payment key (role=0, index=0) for an account from entropy.
fn payment_key_for_account(entropy: &[u8], account: u32) -> XPrv {
    let root = root_key(entropy, b"");
    let acct = account_xprv(&root, account);
    child_xprv(&acct, 0, 0)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Payment key hash (28 bytes) for account N — used as `--admin-key` in `minos publish`.
pub fn payment_key_hash(mnemonic: &Mnemonic, account: u32) -> Hash<28> {
    let entropy = mnemonic.to_entropy();
    vkey_hash(&payment_key_for_account(&entropy, account))
}

/// Enterprise address (payment-only, no staking) for account N.
pub fn enterprise_address(mnemonic: &Mnemonic, account: u32, network_id: u8) -> Address {
    let entropy = mnemonic.to_entropy();
    let pay_key = payment_key_for_account(&entropy, account);
    let pay_hash = vkey_hash(&pay_key);
    let network = if network_id == 1 {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    Address::Shelley(ShelleyAddress::new(
        network,
        ShelleyPaymentPart::key_hash(pay_hash),
        ShelleyDelegationPart::Null,
    ))
}

/// Derive the base-address form using payment 0/0 and stake 2/0 of this account.
pub fn base_address(mnemonic: &Mnemonic, account: u32, network_id: u8) -> Address {
    let account = account_xprv(&root_key(&mnemonic.to_entropy(), b""), account);
    Address::Shelley(ShelleyAddress::new(
        if network_id == 1 {
            Network::Mainnet
        } else {
            Network::Testnet
        },
        ShelleyPaymentPart::key_hash(vkey_hash(&child_xprv(&account, 0, 0))),
        ShelleyDelegationPart::key_hash(vkey_hash(&child_xprv(&account, 2, 0))),
    ))
}

/// Sign the tx body hash with the account N payment key.
/// Returns (vkey_32_bytes, signature_64_bytes).
pub fn sign_tx_body(mnemonic: &Mnemonic, account: u32, body_hash: &[u8]) -> ([u8; 32], [u8; 64]) {
    let entropy = mnemonic.to_entropy();
    let key = payment_key_for_account(&entropy, account);
    let xpub = key.public();
    let sig = key.sign::<()>(body_hash);
    let vk: [u8; 32] = xpub.as_ref()[..32].try_into().unwrap();
    let sig_bytes: [u8; 64] = *sig.to_bytes();
    (vk, sig_bytes)
}

// ---------------------------------------------------------------------------
// Mnemonic I/O
// ---------------------------------------------------------------------------

/// Load a mnemonic from a file. If the path ends in `.gpg`, decrypts via GPG.
pub fn load_mnemonic(path: &std::path::Path) -> Result<Mnemonic> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let text = if ext == "gpg" || ext == "asc" {
        let out = std::process::Command::new("gpg")
            .arg("--decrypt")
            .arg(path)
            .output()
            .context("gpg decrypt: failed to run gpg")?;
        if !out.status.success() {
            anyhow::bail!(
                "gpg decrypt failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        String::from_utf8(out.stdout).context("gpg output is not valid UTF-8")?
    } else {
        std::fs::read_to_string(path).context("read mnemonic file")?
    };
    Mnemonic::parse(text.trim()).map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))
}

/// Save a mnemonic to a file. If `gpg_recipient` is set, encrypts with GPG armor.
pub fn save_mnemonic(
    mnemonic: &Mnemonic,
    path: &std::path::Path,
    gpg_recipient: Option<&str>,
) -> Result<()> {
    let phrase = mnemonic.words().collect::<Vec<_>>().join(" ");
    if let Some(recipient) = gpg_recipient {
        use std::io::Write;
        let mut child = std::process::Command::new("gpg")
            .args(["--encrypt", "--armor", "--recipient", recipient, "--output"])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("gpg encrypt: failed to spawn gpg")?;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(phrase.as_bytes())
            .context("gpg: write mnemonic to stdin")?;
        let status = child.wait().context("gpg encrypt: wait")?;
        if !status.success() {
            anyhow::bail!("gpg encrypt failed");
        }
    } else {
        std::fs::write(path, &phrase).context("write mnemonic file")?;
    }
    Ok(())
}
