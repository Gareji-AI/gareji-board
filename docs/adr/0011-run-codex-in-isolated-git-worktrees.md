# Run Codex in isolated Git worktrees

Status: Accepted

The first Gareji Board Runner Adapter invokes Codex only inside a newly created Git worktree. It never mutates the connected checkout and never falls back to autonomous mutation in a non-Git directory. The Adapter receives one Board-owned execution request and returns one bounded result containing the resolved model, compact handoff, failure category when relevant, and recoverable evidence references.

The Runner validates the Board project, Work item, Agent assignment, Agent capabilities, Execution workspace, instruction reference, and Skill references before starting Codex. It creates a unique `gareji/run-<run-id>` branch at the connected checkout's current `HEAD`, maps a connected subdirectory into the worktree, and invokes Codex with an argument array, `workspace-write` sandboxing, JSONL output, and a bounded handoff schema. It never enables sandbox bypass, Hook-trust bypass, push, or publication.

Run evidence is stored below the configured local Run root rather than in Board coordination data or the connected repository. The first implementation deliberately preserves every worktree. Cleanup requires a later explicit operation after the Progress Checkpoint and evidence are durable and the worktree is clean; automatic cleanup is not part of this decision.

The concrete Adapter is implemented in Board because it owns Codex translation and worktree preparation. Generic capability authorization remains in Gareji Core. Until a published Core crate or accepted execution transport is available, Board does not add a personal path dependency or duplicate Core policy behavior.
