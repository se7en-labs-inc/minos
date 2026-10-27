# Implementation plan

Use clap_complete 4.6.11 with the existing clap 4.6.7 command tree. The documented
stable ahead-of-time generator supports Bash, Zsh, Fish, Elvish and PowerShell.
Add one CLI subcommand, dispatching before workflow validation or construction of
network clients. Generate to stdout, keeping shell setup separate from application
policy. Add file-path value hints for signing/package inputs when appropriate.

Use Nix installShellFiles and installShellCompletion with generated files from the
built Minos command. Keep package input changes focused, preserve runtime wrapping,
notices and existing checks. Update locked dependency/license notices for the new
Apache-2.0/MIT crate without unrelated dependency upgrades.

Verify generation without service access and use Bash to exercise completion of
nested airgap commands, package/wallet options and network enums. Validate Bash
syntax and standard installed paths. Run workspace tests, strict all-target Clippy,
formatting and Nix package/flake checks. New behavior is offline; no live chain
rehearsal is needed. Constitution is satisfied: no fund/key/transaction changes.
