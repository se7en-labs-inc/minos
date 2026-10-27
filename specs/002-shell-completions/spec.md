# Shell completion for Minos

Feature: 002-shell-completions. Requested 2026-10-27. Base: d20ebd1.

## User stories

- As a terminal user, complete Minos commands, nested commands, options and known
  enum values without memorizing spellings. Bash, Zsh and Fish are supported.
- As a Nix user, receive standard shell completion files with the installed
  package; existing shell completion support loads them normally.
- As an airgap user, generate completions entirely offline without wallet access,
  HTTP requests, node connections or dependence on the current network state.

## Requirements and acceptance

1. `minos completions <shell>` writes a script generated from the actual CLI
   definition, including new completion commands and nested airgap/wallet options.
2. Bash, Zsh and Fish output is installable; other generator-supported shells may
   be exposed without promising automatic installation for those shells.
3. Nix installs Bash/Zsh/Fish files in standard share locations. Generation occurs
   at installation using the built executable and requires no runtime network.
4. Bash completion must offer nested subcommands/options and enum values in actual
   shell completion evaluation; unsupported shells fail clearly.
5. Document immediate activation and persistent setup. No existing shell dotfiles,
   wallet files, mainnet signing artifacts, transactions or runtime databases change.

No chain-driven or secret-based suggestions are in scope. Do not query balances,
projects, wallets or keys during completion. This feature authorizes code/package
work, not publication, pushes or live signing/submission.
