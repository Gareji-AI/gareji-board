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

The Tauri desktop Board reads recent accepted checkpoints through Core's bounded local Bridge and converts them into an Activity timeline. It never opens Core SQLite tables. A deep Rust desktop Module owns the bounded read model and explicit mutation intents; Tauri commands adapt that Interface to the embedded local presentation without moving SQL, Runner construction, or policy into JavaScript. Project-only checkpoints appear in the Activity Inbox until a person creates a final Board-owned attachment to a Work item in the same project. The target may be selected from existing Work items or created in `todo` together with the attachment; this atomic Board operation does not rewrite Core's checkpoint or apply its recommendation. Recommended states remain visible suggestions until a person accepts or dismisses them; an accepted supported recommendation and the Board-owned Work item update are stored atomically. The Work item control surface also provides explicit human transitions guarded by the state the person observed, so a stale screen cannot overwrite newer Board state. Delivery failures remain visible per destination and never change Work item state.

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

Gareji Core provides the first Capability Gate, SQLite-backed Progress Recorder, Project Registry, and local Core Bridge. Gareji Board exposes a local Bridge that assesses Work item existence, project relationship, and active-work eligibility from live Board SQLite state. The desktop portfolio reads its coordination cards and canonical-state Kanban lanes from Board SQLite, reads recent Activity through Core, records explicit human transitions, Checkpoint attachments, and reconciliation decisions, and keeps Core-derived facts as Board-owned projections.

Safe Autopilot previews one deterministic candidate and explains every skip before mutation. Its explicit desktop start action pins the Work item to an immutable Control Graph revision and current Agent Loop, resolves the selected Agent profile and Execution workspace, then invokes the Codex Runner Adapter in an isolated Git worktree. Board persists the active Graph position and immutable Route decisions. Runner completion is recorded through Core as one linked Progress Checkpoint, returned as Activity with Core-owned delivery states, and can advance exactly one declared Graph route without changing Work item state. Gate, Audit, Approval, and Terminal stages remain bounded by their evidence and authority rules.

The Tauri desktop includes visual Control Graph, Orchestration Blueprint, and Portfolio orchestration workspaces. Control Graph and Blueprint drafts can add and connect nodes before publishing immutable revisions; graph node positions remain presentation-only state. The headless CLI exposes the same definition rules through atomic JSON edit plans. Portfolio Runs and their steps are durable. Enabled interval schedules use the same finite `PortfolioScheduler` Module from the open desktop, the one-shot CLI, or a local headless daemon. Competing wake-ups compare the latest Run transactionally, and scheduled Portfolio ticks never start a project Runner or bypass a waiting human approval.

One-command first-launch onboarding and deeper Core capability composition remain pending. GBrain, LLMWiki, and a second Runner implementation remain optional future Adapters rather than prerequisites for the local demo.

Board mirrors Core's canonical `progress-checkpoint-v0` transport schema for local validation. The mirrored copies must remain JSON-equivalent; Board does not redefine the Checkpoint contract.
