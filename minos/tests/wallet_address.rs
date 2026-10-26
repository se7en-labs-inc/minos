#[test]
fn base_address_preserves_enterprise_payment_key_and_network() {
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 16]).unwrap();
    for network in [0, 1] {
        for account in [0, 1] {
            let enterprise =
                minos::wallet::enterprise_address(&mnemonic, account, network).to_vec();
            let base = minos::wallet::base_address(&mnemonic, account, network).to_vec();
            assert_eq!(enterprise.len(), 29);
            assert_eq!(base.len(), 57);
            assert_eq!(base[0], network);
            assert_eq!(&enterprise[1..29], &base[1..29]);
            assert_ne!(&base[1..29], &base[29..]);
            assert_eq!(
                hex::encode(&base[1..29]),
                hex::encode(minos::wallet::payment_key_hash(&mnemonic, account))
            );
        }
    }
}
