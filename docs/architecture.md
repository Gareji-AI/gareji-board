# Gareji Board architecture

## Purpose

Gareji Board is the human control surface for understandable multi-project agent work. It owns visible Work item state and portfolio policy while calling narrow Core, Runner, MCP, and Knowledge Adapter Interfaces.

## Suite responsibility map

| Product or Module | Authority |
|---|---|
| Gareji Board | Work item state, priorities, dependencies, assignments, approvals, and portfolio scheduling policy |
| Gareji Core | generic trusted execution, capability enforcement, Compact results, evidence references, Progress Recorder behavior, and local durable intake |
| Runner Adapter | one runtime invocation and its isolated execution environment |
| Gareji MCP | user-enabled Codex transport into existing Core and Board behavior |
| Knowledge Adapter | sourced context reads and independent Progress Checkpoint projection |
| Gareji Cloud | optional identity, synchronization, team administration, and managed capacity |

## Primary flow

```text
Work item + sourced context + policy
  -> Board selects and requests approval
  -> Core validates the bounded execution intent
  -> Runner Adapter executes in an isolated workspace
  -> Core returns a Compact result and evidence references
  -> Progress Recorder stores one immutable Checkpoint
  -> Board reconciles visible state
  -> enabled Knowledge connections receive independent projections
```

Direct human-Codex work enters at the Progress Recorder instead of the Runner. It therefore produces the same Checkpoint and timeline without pretending that Board started the execution.

The desktop Board reads recent accepted checkpoints through Core's bounded local Bridge and converts them into an Activity timeline. It never opens Core SQLite tables. Recommended states remain visible suggestions until a person accepts or dismisses them; an accepted supported recommendation and the Board-owned Work item update are stored atomically. Delivery failures remain visible per destination and never change Work item state.

## Replaceable seams

- Runner: Codex first; another runtime later.
- Knowledge Adapter: local JSON and Markdown Zettelkasten first; GBrain and LLMWiki later.
- Progress capture transport: Runner, MCP, CLI, and trusted lifecycle Hook all call one Recorder.
- Active-work assessment: Core calls the local Board Bridge; Board alone interprets Work item state eligibility.

These seams stay small. Provider fields, delivery retries, conflict rules, and task transitions do not leak into every caller.

## Local-first invariants

- Board and Core remain useful without Cloud.
- Source code remains authoritative in its Execution workspace.
- Work item state remains authoritative in Board.
- Checkpoints are written locally before projection attempts.
- Multiple Knowledge connections preserve source and destination identity.
- Public, destructive, secret-bearing, and production-changing actions require explicit human approval.

## Implementation status

Gareji Core provides the first Capability Gate, SQLite-backed Progress Recorder, Project Registry, and local Core Bridge. Gareji Board exposes a local Bridge that assesses Work item existence, project relationship, and active-work eligibility from live Board SQLite state. The desktop portfolio reads its coordination cards from Board SQLite, reads its recent Activity timeline through Core, and records explicit Checkpoint reconciliation decisions. Board-to-Runner execution and automatic controller reconciliation are not wired yet.

Board mirrors Core's canonical `progress-checkpoint-v0` transport schema for local validation. The mirrored copies must remain JSON-equivalent; Board does not redefine the Checkpoint contract.
