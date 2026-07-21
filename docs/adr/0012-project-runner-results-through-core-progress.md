# Project Runner results through Core Progress

Status: Accepted

Gareji Board converts every started Codex Runner result into one immutable `gareji.progress-checkpoint.v0` record and submits it through the Core Bridge `record_progress` operation. Board does not write a second checkpoint ledger. It returns an immediate Activity projection only after Core accepts the record, using the same Checkpoint fields and per-destination delivery states from Core's receipt.

A policy or preflight rejection is not recorded because no runtime invocation occurred. A successful handoff maps `progress`, `completed`, `blocked`, and `failed` to the corresponding Checkpoint outcome and recommends `in_review`, `done`, `blocked`, and `in_review`. A process failure or timeout records a `failed` outcome and recommends `in_review`. These are recommendations only; recording never changes the Board-owned Work item state.

The Checkpoint contains relative changed paths, final Git revision, Run branch, dirty state, bounded verification summaries, and opaque `run://` evidence references. It does not contain the local worktree path, raw JSONL events, full diff, credentials, prompt, or personal workspace path. The stable `checkpoint-<run-id>` identity makes retries idempotent through Core.
