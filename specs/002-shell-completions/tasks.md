# Tasks

- [x] T001 Inspect branch/worktree/spec numbers and create verified sibling 002;
  record specification and plan before implementation.
- [x] T002 Add locked completion generation from the actual Clap command tree,
  dispatch entirely offline and expose useful path/enum completion hints.
- [x] T003 Install Bash/Zsh/Fish completions through the Nix package and document
  shell activation; preserve existing runtime and license packaging.
- [x] T004 Validate real Bash completion, invalid-shell errors, offline behavior,
  required workspace/lint/format/package gates and dependency notice update.

Validation: 155 workspace Rust tests, strict all-target Clippy, formatting and
all flake checks pass. Built Nix package installs nonempty Bash/Zsh/Fish scripts;
installed Bash registration and offline generation pass. Interactive Bash Tab
completion and Zsh/Fish syntax checks pass. Locked license inventory regenerated.
