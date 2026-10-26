# Tasks

- [x] T001 Add optional comments parsing and rendering to both generation modes.
- [x] T002 Add Christina's text to daedalus-project.md and document the section.
- [x] T003 Check compilation/formatting and inspect generated DOCX comments,
  including backward compatibility when the section is omitted.

Validation: cargo check -p minos and rustfmt --check passed through nix develop
(existing unused-code warnings only). Generated
MAF-M2.1-Sep2026-With-Assurer-Comments.docx from the published September report;
read back both supplied paragraphs under the comments heading. Separate outputs
with absent and empty sections retained the same blank comments area. GitHub
mode wiring was compiled; GitHub fetching was not exercised.
