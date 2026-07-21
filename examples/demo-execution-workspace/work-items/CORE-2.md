# CORE-2: Add active-work registry

Create a small, local registry that makes the demo's currently active work explicit.

## Acceptance criteria

- Add `config/active-work-registry.json` as valid UTF-8 JSON.
- Use exactly two top-level fields: `schema_version` with the number `1`, and `entries` with an array.
- Add these entries in this order, with no extra fields:
  1. `gareji-board`, `BOARD-1`, `in_progress`
  2. `gareji-core`, `CORE-1`, `in_review`
- Each entry uses the field names `project_id`, `work_item_id`, and `state`.
- Add an `Active-work registry` section to the workspace `README.md` that links to the JSON file and says it is demo-only local data with no network or publication behavior.
- Verify that the JSON parses and that its exact shape and values match this specification.

Keep the change limited to the registry and README. Do not add dependencies, network access, publication, or automation.
