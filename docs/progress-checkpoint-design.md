# Progress Checkpoint design

Progress Checkpoint makes Runner work and direct project work equally visible. Gareji Board records one immutable event, then derives its activity timeline and zero or more knowledge-workspace notes from that event. It does not perform bidirectional task-state synchronization.

## Decision

- Gareji Board remains authoritative for Work item state.
- The execution workspace remains authoritative for files, Git state, and test results.
- The Core Progress Recorder's checkpoint ledger is authoritative for captured progress events.
- Generated progress notes are append-only projections, not additional writable task stores.
- Every destination has its own eventually consistent delivery with `pending`, `synced`, `conflict`, or `failed`.

## Capture sources

All sources call the same Progress Recorder Module:

| Source | v0 behavior |
|---|---|
| Runner | Record after a bounded Run result or reconciliation |
| MCP `record_progress` | Record a structured checkpoint supplied by an enabled agent or Skill |
| `gareji checkpoint` | Let a person save meaningful intermediate work explicitly |
| Codex `Stop` Hook | Record once at turn end when a connected workspace has a material change or structured handoff |
| Git `post-commit` Hook | Deferred optional adapter for work outside Codex |

`PostToolUse` is not a persistence trigger because it would create noisy events for individual commands and edits. The `Stop` Hook is a safety net, not the source of task truth. It must not depend on the transcript format; it uses stable Hook identifiers, the current working directory, Git evidence, and any structured handoff produced during the turn.

## Capture frequency

The default `turn_end_material` policy records at most one checkpoint when a Codex turn ends and at least one of these is true:

- tracked files changed;
- verification ran;
- a blocker, failure, completion, or explicit handoff was produced;
- the recommended Work item state changed;
- the user requested a checkpoint.

No-op turns and duplicate fingerprints are ignored. A `milestone_only` policy is also supported for users who prefer lower frequency: explicit checkpoint, Git commit, merge, or a later remote PR event. Local Git has reliable `post-commit` and `post-merge` points; push completion and PR merge require a remote-provider event and are deferred beyond v0.

## Progress Recorder Module

The Module exposes two operations:

```text
record(checkpoint) -> receipt
sync_pending() -> delivery summary
```

Core also exposes a bounded newest-first history read through its local Bridge. Board uses that Interface to build the Activity timeline and never queries the Recorder's SQLite tables directly. The history result includes current per-destination delivery state and bounded failure summaries so reconciliation remains understandable while the ledger stays Core-owned.

Validation, workspace resolution, redaction, idempotency, durable outbox writes, Activity Inbox routing, Board projection, knowledge projection, retries, and conflict detection remain inside the Module. Runner, MCP, CLI, and Hook adapters do not reproduce these rules.

The only MCP tool that creates progress is `record_progress`. MCP does not expose a capture-time `complete_work_item` shortcut. A checkpoint carries `recommended_state`; Gareji Board applies or requests approval for the actual transition.

The first Board approval flow records one final reconciliation decision per linked Checkpoint. An accepted supported recommendation and its Work item transition commit together; a dismissed recommendation leaves state unchanged. Core's Checkpoint is never edited, and correcting a past decision uses a separate Work item action so the evidence trail remains understandable.

## Task linking

The preferred link is an explicit Work item selected in Gareji Board or passed to `gareji checkpoint`. If no Work item is supplied, the Recorder resolves the connected execution workspace and places the checkpoint in that project's Activity Inbox.

The Activity Inbox lists these project-only checkpoints separately from linked Activity. A person may attach one to an existing Work item in the same Board project. Board stores that final association as a projection keyed by Checkpoint ID, while the Core-owned Checkpoint keeps its original empty `work_item_id`; identical retries are idempotent and a later different target is rejected.

`gareji task start <work-item-id>` stores the active Work item for the connected Execution workspace. The active selection remains available while Gareji Board is closed and is cleared explicitly or when the Work item reaches a terminal state.

Branch names and text matches may be shown as suggestions but never silently assign a Work item. If no connected execution workspace matches the current directory, recording fails locally with a bounded diagnostic and does not write into an unrelated project.

## Idempotency and conflicts

- Assign one `checkpoint_id` before the durable outbox write.
- Retry every destination with the same ID and payload.
- Treat the same ID and same payload as success.
- Treat the same ID with a different payload as `conflict`; never overwrite either copy.
- Use a content fingerprint to suppress accidental near-duplicates from the same turn or Git state.
- Write the checkpoint locally before attempting any projection.

## Local storage and availability

The Core Progress Recorder stores the active Work item reference, immutable checkpoint ledger, outbox, and delivery receipts in SQLite under the operating system's application-data location. Board supplies project and workspace mappings through an Interface and consumes the same operational records for display. Board-owned coordination records and Core-owned operational records remain separate even if they share one physical database. Gareji does not place this state inside an Execution workspace or Knowledge workspace. JSON is reserved for fixtures, import/export, and the bundled `local_json` Knowledge Adapter; it is not the live control database.

Runner, MCP, CLI, and Hook adapters write through the Recorder directly, so capture works while Gareji Board is closed. The v0 design does not require an always-on daemon; Board consumes the same local state when it next opens.

## Knowledge-workspace projection

A Board project may have several Knowledge connections enabled simultaneously. The Recorder reads context across all enabled read connections while preserving each source identity. It does not silently merge conflicting source facts.

For every enabled progress-write connection, the Recorder creates an independent delivery for the same immutable checkpoint ID. If the project exists in both a Markdown Zettelkasten and another writable knowledge backend, both receive a progress projection. Success in one destination is retained when another is unavailable; retry targets only unsynced deliveries. The Board shows `fully synced` only when every required destination is `synced`, and otherwise shows the per-destination result.

The Markdown adapter writes one generated note per checkpoint under a configured project progress directory, for example `4_Project/Gareji-Board/Progress/2026/07/<checkpoint_id>.md`. One-file-per-event avoids concurrent append conflicts and preserves history.

The note contains the checkpoint ID, time, source, outcome, summary, changed paths, verification summary, evidence references, risks, and next action. It does not contain a raw transcript, full diff, credentials, or full execution log.

Local note creation and Git publication are separate actions. Commit and push remain manual in v0; later policies may explicitly allow batched or private automatic publication.

## Knowledge Adapter seam

Progress Recorder invokes one stable operation for each enabled write connection:

```text
project_checkpoint(workspace, checkpoint) -> delivery receipt
```

The bundled demo implements this seam twice and uses both implementations together: `zettelkasten_markdown` as the first real adapter and `local_json` as a dependency-free adapter. GBrain and LLMWiki are later Adapters; they do not add provider-specific fields to Progress Checkpoint or Work item.

## Failure behavior

An unavailable knowledge workspace does not lose work or block development. The local checkpoint remains durable, successful destinations remain synced, and the Board shows the unavailable destination as `pending` or `failed`. Retry resumes that destination from the same ID. A conflicting destination becomes `conflict` and requires review.

No delivery failure changes the Work item state automatically. A `failed` execution outcome normally recommends `in_review`; an explicit blocker may recommend `blocked`; verified completion or no-action may recommend `done`.

## Privacy

Capture only compact summaries, relative changed paths, Git references, verification results, and evidence links. Redact likely secrets before the durable write. Keep raw logs and detailed evidence in local sidecars, and never scan unrelated knowledge-workspace content to enrich a checkpoint.
