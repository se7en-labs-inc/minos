#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{symlink, DirBuilderExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "minos-wallet-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn wallet_new(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minos"));
        command.args(["wallet", "new"]);
        command.env("GNUPGHOME", self.0.join("gnupg"));
        command
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if self.0.join("gnupg").exists() {
            let _ = Command::new("gpgconf")
                .env("GNUPGHOME", self.0.join("gnupg"))
                .args(["--kill", "gpg-agent"])
                .output();
        }
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn plaintext_wallet_is_private_and_existing_paths_are_preserved() {
    let dir = TestDir::new();
    let path = dir.0.join("wallet");
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 32]).unwrap();
    minos::wallet::save_mnemonic(&mnemonic, &path, None).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(minos::wallet::load_mnemonic(&path).unwrap(), mnemonic);
    let original = fs::read(&path).unwrap();
    let other = bip39::Mnemonic::from_entropy(&[1; 32]).unwrap();
    assert!(minos::wallet::save_mnemonic(&other, &path, None).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(minos::wallet::save_mnemonic(&other, &dir.0, None).is_err());
    assert!(minos::wallet::save_mnemonic(&other, &dir.0.join("missing/wallet"), None).is_err());
}

#[test]
fn wallet_creation_refuses_existing_and_dangling_symlinks() {
    let dir = TestDir::new();
    let target = dir.0.join("target");
    fs::write(&target, "keep existing contents").unwrap();
    let link = dir.0.join("link");
    symlink(&target, &link).unwrap();
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 32]).unwrap();
    assert!(minos::wallet::save_mnemonic(&mnemonic, &link, None).is_err());
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "keep existing contents"
    );
    let missing = dir.0.join("missing");
    let dangling = dir.0.join("dangling");
    symlink(&missing, &dangling).unwrap();
    assert!(minos::wallet::save_mnemonic(&mnemonic, &dangling, None).is_err());
    assert!(!missing.exists());
}

#[test]
fn cli_saved_mnemonic_is_never_implicitly_displayed() {
    let dir = TestDir::new();
    let path = dir.0.join("wallet");
    let output = dir.wallet_new().arg("--out").arg(&path).output().unwrap();
    assert_success(&output);
    let phrase = fs::read_to_string(&path).unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&phrase));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&phrase));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("Saved to "));
    let repeat = dir.wallet_new().arg("--out").arg(&path).output().unwrap();
    assert!(!repeat.status.success());
    assert!(repeat.stdout.is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), phrase);
}

#[test]
fn cli_requires_explicit_display_and_can_opt_in_when_saving() {
    let dir = TestDir::new();
    let implicit = dir.wallet_new().output().unwrap();
    assert!(!implicit.status.success());
    assert!(implicit.stdout.is_empty());
    let explicit = dir.wallet_new().arg("--show-mnemonic").output().unwrap();
    assert_success(&explicit);
    assert!(bip39::Mnemonic::parse(String::from_utf8(explicit.stdout).unwrap().trim()).is_ok());
    let path = dir.0.join("wallet");
    let saved = dir
        .wallet_new()
        .args(["--show-mnemonic", "--out"])
        .arg(&path)
        .output()
        .unwrap();
    assert_success(&saved);
    assert!(String::from_utf8(saved.stdout)
        .unwrap()
        .contains(&fs::read_to_string(path).unwrap()));
}

#[test]
fn gpg_wallet_is_private_encrypted_and_absent_on_encryption_failure() {
    let dir = TestDir::new();
    let gnupg = dir.0.join("gnupg");
    fs::DirBuilder::new().mode(0o700).create(&gnupg).unwrap();
    let generated = Command::new("gpg")
        .env("GNUPGHOME", &gnupg)
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-generate-key",
            "Minos Wallet Test <wallet-test@example.invalid>",
            "future-default",
            "default",
            "0",
        ])
        .output()
        .unwrap();
    assert_success(&generated);
    let path = dir.0.join("wallet.asc");
    let output = dir
        .wallet_new()
        .args(["--gpg-recipient", "wallet-test@example.invalid", "--out"])
        .arg(&path)
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let encrypted = fs::read(&path).unwrap();
    assert!(encrypted.starts_with(b"-----BEGIN PGP MESSAGE-----"));
    let decrypted = Command::new("gpg")
        .env("GNUPGHOME", &gnupg)
        .arg("--decrypt")
        .arg(&path)
        .output()
        .unwrap();
    assert_success(&decrypted);
    let phrase = String::from_utf8(decrypted.stdout).unwrap();
    assert!(bip39::Mnemonic::parse(&phrase).is_ok());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&phrase));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&phrase));
    let repeat = dir
        .wallet_new()
        .args(["--gpg-recipient", "wallet-test@example.invalid", "--out"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!repeat.status.success());
    assert_eq!(fs::read(&path).unwrap(), encrypted);
    let failed_path = dir.0.join("failed.asc");
    let failed = dir
        .wallet_new()
        .args(["--gpg-recipient", "missing@example.invalid", "--out"])
        .arg(&failed_path)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(!failed_path.exists());
}
