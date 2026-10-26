# MAF third party assurer comments

As a MAF author, I can include an assurer's supplied comments in the project
Markdown so regeneration retains them in the dedicated comments section.

Requirements:
- Read optional `## Third Party Assurer Comments` as Markdown.
- Render its paragraphs beneath the existing comments heading in both report
  and GitHub generation modes, using the existing narrative formatting.
- Preserve the existing blank comments area when the section is absent or empty.
- Store Christina's supplied M2.1 September 2026 acceptance text verbatim in
  `daedalus-project.md`; do not add a signature or acceptance date.
- Preserve existing project fields and uncommitted messaging changes.

Acceptance: generate a MAF from the project file and verify both supplied
paragraphs appear between the comments and signature headings. Generate with
the section omitted and confirm the comments area remains blank.

Scope excludes remote document edits, signing and milestone submission.
