use minos::{
    invoice::{self, Package, SignedPackage, UnlockPayload},
    wallet,
};
use std::process::Command;

#[test]
fn invoice_cli_reviews_signs_and_verifies_offline_with_a_public_test_wallet() {
    let root = std::env::temp_dir().join(format!("minos-invoice-cli-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    // Public synthetic mnemonic, never an operator wallet.
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 32]).unwrap();
    let package = Package::unlock(
        1,
        hex::encode(wallet::payment_key_hash(&mnemonic, 0).unwrap()),
        UnlockPayload {
            instance: "a".repeat(56),
            project: "TEST".into(),
            nonce: "00".repeat(16),
            timestamp: "2000-01-01T00:00:00Z".into(), // Offline signing ignores wall clock.
        },
    )
    .unwrap();
    let request = root.join("request.json");
    let response = root.join("signed.json");
    let test_wallet = root.join("public-test-wallet.txt");
    invoice::write_private(&request, &package).unwrap();
    std::fs::write(&test_wallet, mnemonic.to_string()).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_minos"))
            .args([
                "--portal",
                "invalid-url",
                "--node-socket",
                "/nonexistent/invoice.socket",
                "invoice",
            ])
            .args(args)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("KOIOS_API_KEY", "invoice-environment-secret")
            .output()
            .unwrap()
    };
    let review = run(&["review", "--package", request.to_str().unwrap()]);
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    assert!(!String::from_utf8_lossy(&review.stdout).contains("invoice-environment-secret"));
    let output = run(&[
        "sign",
        "--package",
        request.to_str().unwrap(),
        "--wallet",
        test_wallet.to_str().unwrap(),
        "--out",
        response.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let signed: SignedPackage = invoice::read_private(&response).unwrap();
    package.verify(&signed).unwrap();
    let verified = run(&[
        "verify",
        "--package",
        request.to_str().unwrap(),
        "--signed",
        response.to_str().unwrap(),
    ]);
    assert!(verified.status.success());
    let existing = std::fs::read(&response).unwrap();
    let repeat = run(&[
        "sign",
        "--package",
        request.to_str().unwrap(),
        "--wallet",
        test_wallet.to_str().unwrap(),
        "--out",
        response.to_str().unwrap(),
    ]);
    assert!(!repeat.status.success());
    assert_eq!(std::fs::read(&response).unwrap(), existing);
    let wrong = run(&[
        "sign",
        "--package",
        request.to_str().unwrap(),
        "--wallet",
        "absent-wallet.asc",
        "--expected-request-hash",
        &"00".repeat(32),
        "--out",
        root.join("wrong.json").to_str().unwrap(),
    ]);
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("hash mismatch"));
    assert!(!root.join("wrong.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}
