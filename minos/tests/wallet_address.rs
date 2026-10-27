#[test]
fn published_ed25519_signature_vector() {
    use pallas_crypto::key::ed25519::{PublicKey, SecretKey, Signature};
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wallet/ed25519.json")).unwrap();
    let seed: [u8; 32] = hex::decode(vector["seed"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let public: [u8; 32] = hex::decode(vector["public_key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let signature: [u8; 64] = hex::decode(vector["signature"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let message = hex::decode(vector["message"].as_str().unwrap()).unwrap();
    let actual = SecretKey::from(seed).sign(&message);
    assert_eq!(actual, Signature::from(signature));
    assert!(PublicKey::from(public).verify(&message, &actual));
}

#[test]
fn base_address_preserves_enterprise_payment_key_and_network() {
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 16]).unwrap();
    for network in [0, 1] {
        for account in [0, 1] {
            let enterprise = minos::wallet::enterprise_address(&mnemonic, account, network)
                .unwrap()
                .to_vec();
            let base = minos::wallet::base_address(&mnemonic, account, network)
                .unwrap()
                .to_vec();
            assert_eq!(enterprise.len(), 29);
            assert_eq!(base.len(), 57);
            assert_eq!(base[0], network);
            assert_eq!(&enterprise[1..29], &base[1..29]);
            assert_ne!(&base[1..29], &base[29..]);
            assert_eq!(
                hex::encode(&base[1..29]),
                hex::encode(minos::wallet::payment_key_hash(&mnemonic, account).unwrap())
            );
        }
    }
}

#[test]
fn published_shelley_address_and_signing_key_match() {
    use pallas_crypto::{
        hash::Hasher,
        key::ed25519::{PublicKey, Signature},
    };
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wallet/shelley.json")).unwrap();
    let mnemonic = bip39::Mnemonic::parse(vector["mnemonic"].as_str().unwrap()).unwrap();
    let enterprise = minos::wallet::enterprise_address(&mnemonic, 0, 0).unwrap();
    assert_eq!(enterprise.to_bech32().unwrap(), vector["enterprise"]);
    assert_eq!(
        minos::wallet::base_address(&mnemonic, 0, 0)
            .unwrap()
            .to_bech32()
            .unwrap(),
        vector["base"]
    );
    let message = [42; 32];
    let (key, signature) = minos::wallet::sign_tx_body(&mnemonic, 0, &message).unwrap();
    assert_eq!(
        Hasher::<224>::hash(&key).as_ref(),
        &enterprise.to_vec()[1..]
    );
    assert!(PublicKey::from(key).verify(message, &Signature::from(signature)));
    assert!(!PublicKey::from(key).verify([43; 32], &Signature::from(signature)));
    assert_eq!(
        minos::wallet::sign_tx_body(&mnemonic, 0, &message).unwrap(),
        (key, signature)
    );
    assert_eq!(
        &minos::wallet::enterprise_address(&mnemonic, 0, 1)
            .unwrap()
            .to_vec()[1..],
        &enterprise.to_vec()[1..]
    );
}

#[test]
fn account_bounds_and_unsupported_networks_fail_instead_of_aliasing() {
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 16]).unwrap();
    for account in [0x80000000, u32::MAX] {
        assert!(minos::wallet::payment_key_hash(&mnemonic, account).is_err());
        assert!(minos::wallet::enterprise_address(&mnemonic, account, 0).is_err());
        assert!(minos::wallet::base_address(&mnemonic, account, 0).is_err());
        assert!(minos::wallet::sign_tx_body(&mnemonic, account, &[0; 32]).is_err());
    }
    assert!(minos::wallet::payment_key_hash(&mnemonic, 0x7fffffff).is_ok());
    assert!(minos::wallet::enterprise_address(&mnemonic, 0, 2).is_err());
    assert!(minos::wallet::base_address(&mnemonic, 0, 2).is_err());
}
