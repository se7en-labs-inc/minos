# Implementation plan

Extend ProjectConfig and MafOptions in minos/src/maf.rs with the optional
comments narrative. Parse it using the existing section map and convert it using
markdown_to_paras in both commands, including existing month substitution.
Populate the existing full-width comments cell, retaining five blank paragraphs
when no comments are supplied. Add the supplied text to daedalus-project.md and
document the optional section in README.md.

Use the existing worktree to preserve the in-progress MAF edits. Follow the
repository's existing unnumbered spec layout; no feature branch is allocated.
Validate through the Nix dev shell with cargo check, formatting, and generated
DOCX XML inspection for populated and omitted comments. Use the published
September report for the uploadable output; preserve existing DOCX files.
