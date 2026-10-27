# History sanitation review

The feature was rebased onto master at `27ba8f8`. On 2026-10-27 the maintainer
authorized local integration of the sanitized feature as master. Original history
and modified operational notes are preserved in a private external backup;
local wallet/transaction/report files remain untouched. No remote operation or
publication is included.

The separate, disposable release candidate preserves all 17 original commits,
including commits made empty by removal of private notes, plus a feature snapshot
and a restoration commit, followed by the local-notes exclusion. Author/committer name and email
sets are preserved without mailmapping. Gitleaks reports no secrets. Reachable
commit counts and commits containing changes can differ because empty commits
are preserved for attribution.

## Historical content removed

| Historical path | Finding | Treatment in the candidate |
|---|---|---|
| `CONTEXT.md` | Private local session notes | Remove from every commit, never restore to tracked source; ignore the local file |
| `README.md` | Personal node socket paths and deployment context | Remove historical versions; restore current portable README |
| `specs/pallas-node-backend/handoff.md` | Personal checkout/cache paths and operational handoff | Remove historical versions; restore current portable handoff |
| `specs/portal-airgap-completion/` | Legacy operational wallet/project context | Remove historical versions; restore current portable artifacts |
| `specs/preview-cli-lifecycle/` | Legacy operational deployment context | Remove historical versions; restore current portable artifacts |
| `minos-portal/minos-portal.example.toml` | Historical deployment examples | Remove historical versions; restore current generic example |

Personal home prefixes in other historical blobs and commit messages are replaced
with an example home prefix. Approved author/email attribution and the Nix
maintainer handle remain. Secret detection alone does not establish privacy;
historical content and the final source receive separate personal-path reviews.

## Reproduction and verification

1. Capture explicitly tracked/intended feature paths in a disposable bare copy,
   with an alternate index. Do not add ignored wallet/report/transaction artifacts.
2. Run Nix-provided `git-filter-repo` only against that disposable copy. Remove
   the historical paths above, replace personal home prefixes, and disable empty
   and degenerate commit pruning to preserve original commit attribution.
3. Restore only captured portable paths other than `CONTEXT.md`. That file must
   remain ignored and absent from every reachable commit. Assert the resulting tree
   equals the captured feature tree exactly and author/email sets match.
4. Run `git fsck`, verify a self-contained branch bundle, scan all candidate refs
   with Gitleaks and inspect all reachable trees for personal home paths. Build
   packages and run flake checks against the exact committed candidate Git URI.
5. Retain the bundle, commit map and review metadata in private task artifacts.
   Export only the sanitized bundle, never the whole scratch directory containing
   the original private clone metadata.

The prepared candidate passes these checks. Final evidence updates require a fresh
snapshot and repeat checks. Applying the candidate to actual repository refs or
publishing it requires a separate maintainer decision; preparing a bundle does
not rewrite the original repository or authorize a force-push.
