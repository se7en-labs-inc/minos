use std::process::Command;

fn generate(shell: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minos"))
        .args(["--portal", "invalid-url", "completions", shell])
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env(
            "CARDANO_NODE_SOCKET_PATH",
            "/nonexistent/completion-test.socket",
        )
        .env("KOIOS_API_KEY", "completion-secret-sentinel")
        .output()
        .unwrap()
}

#[test]
fn completions_generate_offline_for_supported_shells_without_exposing_environment() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let output = generate(shell);
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(!script.is_empty());
        assert!(script.contains("minos"));
        assert!(!script.contains("completion-secret-sentinel"));
        assert!(!script.contains("/nonexistent/completion-test.socket"));
    }
    assert!(!generate("not-a-shell").status.success());
}

#[test]
fn bash_completes_nested_commands_options_enum_values_and_paths() {
    let dir = std::env::temp_dir().join(format!("minos-completion-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("minos.bash");
    std::fs::write(&script, generate("bash").stdout).unwrap();
    std::fs::write(
        dir.join("completion wallet.asc"),
        "public completion fixture",
    )
    .unwrap();
    assert!(Command::new("bash")
        .arg("-n")
        .arg(&script)
        .status()
        .unwrap()
        .success());
    let complete = |words: &[&str]| {
        let result = Command::new("bash")
            .args([
                "-c",
                r#"
# Headless invocation can generate candidates, but compopt's interactive display
# settings are unavailable; generated functions handle that and return success.
source "$1" || exit 1
shift
COMP_WORDS=("$@")
COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
_minos minos "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"
printf '%s\n' "${COMPREPLY[@]}"
"#,
                "completion-test",
            ])
            .arg(&script)
            .args(words)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let root = complete(&["minos", "ai"]);
    assert!(root.contains(&"airgap".into()), "root candidates: {root:?}");
    let airgap = complete(&["minos", "airgap", ""]);
    for subcommand in ["review", "sign", "verify"] {
        assert!(airgap.contains(&subcommand.into()));
    }
    assert!(complete(&["minos", "airgap", "sign", "--p"]).contains(&"--package".into()));
    let invoices = complete(&["minos", "invoice", ""]);
    for command in [
        "prepare-package",
        "sign",
        "upload-package",
        "confirm",
        "unlock",
        "list",
        "download",
    ] {
        assert!(
            invoices.contains(&command.into()),
            "invoice candidates: {invoices:?}"
        );
    }
    assert!(
        complete(&["minos", "invoice", "prepare-package", "--invoice-d"])
            .contains(&"--invoice-date".into())
    );
    let networks = complete(&["minos", "--network", ""]);
    for network in ["preview", "mainnet"] {
        assert!(networks.contains(&network.into()));
    }
    assert!(
        complete(&["minos", "airgap", "sign", "--wallet", "completion"])
            .contains(&"completion wallet.asc".into())
    );
    std::fs::remove_dir_all(dir).unwrap();
}
