# ADR 0018: Edit immutable definitions through atomic CLI plans

## Status

Accepted

## Context

The Tauri workspaces can edit Control Graph and Orchestration Blueprint drafts, but a
headless operator also needs to inspect and configure nodes and connections. Saving
each CLI operation immediately is unsafe because an added node may be temporarily
unreachable until a later connection is added.

Accepting an arbitrary replacement document would also duplicate validation logic and
make it easy for a caller to mistake an immutable revision for mutable configuration.

## Decision

Board exposes `control-graph` and `blueprint` CLI command groups with `list`, `show`,
and `apply` operations. `apply` accepts one bounded JSON edit plan containing:

- the exact source definition and revision identity;
- a distinct new immutable revision identity; and
- an ordered list of node and connection edits.

The `DefinitionEditor` Module loads the exact source, applies every operation through
`GraphDraft` or `BlueprintDraft`, validates the completed topology, and performs one
immutable revision write. Temporary incomplete topology remains inside the Module.
Plans are limited to 1 MiB and can be read from a file or standard input.

## Consequences

- CLI and desktop edits use the same domain validation and immutable storage rules.
- Multi-step topology changes either publish one valid revision or write nothing.
- Existing revisions are never mutated in place.
- JSON plans are intentionally explicit and suitable for review, source control, and
  generation by other local tools.
