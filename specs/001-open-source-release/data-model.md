# Data model and invariants

This describes logical records, not a premature choice of storage engine.

| Entity | Identity / essential data | Invariant |
|---|---|---|
| Network | network magic plus address network mapping | No URL-based silent network substitution |
| Instance | network + registry policy ID | Script references/hashes validated against registry |
| Project | instance key + project identifier | Same identifier in another instance cannot collide |
| Vendor output | network + transaction hash + output index | Full assets, datum, chain point, and spent/unspent status |
| Output lineage | consumed output → produced output(s), validated event | Project association follows evidence, not payout counts |
| Milestone association | project + metadata identifier + validated payout mapping | Ambiguity recorded explicitly; no assumed sorted prefix |
| Chain event | chain point + tx hash + event discriminator + instance/project | Provenance and valid transaction effects verified before projection |
| Registration | transaction body hash + signed envelope + local label + time | Pending local record is distinct from confirmed event |
| Projection | versioned instance/project/milestone/UTxO state | Deterministically rebuildable from accepted events |
| Checkpoint | network/stream identity + slot + block hash | Written atomically only after associated durable work |
| Schema version | format version and migration state | Unknown/newer schemas fail clearly; no destructive guessing |
| Transaction value | policy/name → checked quantity, including ADA | Inputs + mint + withdrawals balance outputs + fees/deposits |
| Verified reference | output reference + language + bytes + expected hash | Missing/mismatched scripts fail before transaction output |

## State transitions

Observed metadata → provenance-validated event → durable journal → projected state.
Rejected or unresolved observations do not become confirmed state. Registration may
later correlate with chain confirmation; registration alone never advances it.
Rollback reverses or rebuilds projections, accepted events and replay markers after
the rollback point; origin clears the relevant chain projection/checkpoint only.
Reset is scoped to the requested instance and documented regarding shared streams.

## Migration decision

Research must settle authoritative storage and legacy associations before schema
implementation. If old records lack recoverable lineage, require an explicit
backed-up chain reindex and retain local registrations separately. Migration must
be restartable and tested with duplicate project IDs, multiple instances, and
partially completed writes. Exact schema is recorded when this gate is resolved.

## Storage implementation decision — schema 3

Retain embedded Sled and DuckDB. Sled is the authoritative durable journal and
projection store; DuckDB is a rebuildable event-query projection. A single Sled
transaction must commit an accepted block's journal, projected documents, scoped
dedup markers and slot/hash checkpoint. Flush Sled before reporting durable
success. Apply/rebuild DuckDB after that commit; a crash in that window is repaired
from the journal before serving queries. DuckDB never independently decides
whether an event was applied. No separate database service is needed.

Each database belongs to one configured network. Store schema version and network
in an explicit metadata tree and reject reopening it under another network or
newer schema. Projects and current vendor outputs use a serialized tuple
`[instance_id, project_id]`, not a delimiter-concatenated or project-only key.
Schema 2's independently written legacy projection keys are migrated explicitly.
Schema 3 keeps each instance's canonical state/events/dedup together in one
projection record, raw relevant blocks in ordered journal keys, and a whole-block
checkpoint in the same default-tree atomic batch. Dedup is the per-instance
canonical set of transaction body hashes; event identity includes instance,
transaction, event discriminator and project. A project's local identifier may
contain punctuation without key ambiguity. Output identity remains tx hash/index.

Schema-less stores must not be guessed or mutated during normal startup. The
explicit migration procedure first backs up both stores, preserves registered
transactions and tracked instances, then rebuilds chain-derived state from the
network journal/origin; legacy payout associations without evidence become
unresolved. An interrupted migration remains detectable and restartable.

Rollback/reset rebuild only the affected instance from retained accepted journal
entries; origin retains none for that instance. Clear that instance's checkpoint,
dedup, project/output/event projections, including final consumed-output markers.
Retain separate local transaction registrations and tracked-instance settings.
Readers must not observe partially rebuilt state. Snapshot polling is an
observation of availability, never authority to create a confirmed event or infer
which milestones were withdrawn. Query failure cannot act as an empty snapshot.

## Lineage and milestone association decision

Bind initial project identity to a provenance-validated funding transaction and its
vendor output. Exactly one eligible output plus an explicit ordered milestone list
can establish the initial payout-index mapping. Multiple eligible outputs require
an explicit validated association; otherwise expose an unresolved observation.
Legacy milestone maps are unordered and must not be sorted to fabricate payout
order. They can be resolved only with unique metadata/datum evidence; ambiguity
remains visible and can be repaired by explicit backed-up reindex with evidence.

Subsequent identity follows consumed vendor outrefs. A new datum's payouts match
prior payouts by complete value and maturation, with the authorized transition
permitting only its specified status changes. Preserve remaining order; withdrawal
removes matured Active entries even when those entries are not a prefix. Duplicate
identical payouts need distinguishable lineage/metadata evidence or remain
unresolved. Modify may reorder or replace payouts; apply explicit unique identity
mapping, and expose unmapped milestones as unresolved. No nearest payout-count,
lexical identifier, or paused-position guess is accepted. A finally consumed
vendor output is recorded even when no continuing output exists.
