# Gareji MCP v0 design

Gareji MCP is a thin, user-enabled transport into Gareji Core's project, context, active-work, and Progress Recorder modules. It does not own Board state or reproduce storage, redaction, deduplication, delivery, or approval rules.

## Tools

```text
list_projects() -> projects
get_project_context(project_id, work_item_id?) -> sourced context
set_active_work_item(project_id, work_item_id) -> active selection receipt
record_progress(checkpoint) -> checkpoint receipt
get_checkpoint_status(checkpoint_id) -> per-destination delivery status
```

`get_project_context` reads all enabled Knowledge connections and preserves the workspace and source identity of every result. `record_progress` writes the immutable checkpoint locally first, then creates one delivery per enabled progress-write connection. `get_checkpoint_status` reports partial success rather than collapsing several destinations into one Boolean result.

## Permission boundary

The integration must be installed, enabled, and trusted by the user. Each project separately grants context-read, active-work selection, and progress-write permission. The v0 interface exposes no arbitrary file write, deletion, shell execution, Git commit, Git push, publication, secret access, or direct Work item completion tool.

Gareji Core owns the tool behavior and SQLite-backed state. The MCP package owns protocol translation and may bundle the Codex configuration, trusted `Stop` Hook, and sample Skills for the hackathon. Gareji Board owns the UI and applies any recommended Work item state under its approval policy.
