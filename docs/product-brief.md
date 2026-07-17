# Gareji Board product brief

## User problem

Work with AI agents becomes opaque quickly. A person cannot easily tell what is waiting, which agent is responsible, whether an agent is actually progressing, why it failed, or what needs human judgment.

## Product thesis

Gareji Board makes an autonomous work loop understandable without requiring the user to write orchestration code.

```text
work item + context + policy
  -> autopilot selects one safe next action
  -> selected custom agent runs through Gareji Core
  -> compact result, evidence, and next action return to the board
  -> human reviews only the decisions that need judgment
```

## Visible promises

### 1. Autopilot that is understandable

The Board does not merely show that a schedule ran. It shows why an item was selected, which policy or safety gate applied, and why other items were skipped.

### 2. Custom agents, not one black-box assistant

Users choose or define agent profiles such as `researcher`, `implementer`, `reviewer`, or `release checker`. A profile has a role, capabilities, allowed actions, budget/time limits, an optional model override, and a trusted Core plugin binding. Model choice is independent from instructions and skills, so the same agent can run on a different Codex model without being redefined. The Board never treats a free-form shell command as an agent definition.

### 3. State and failure are first-class

Every work item uses the Gareji lifecycle: `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, or `cancelled`. Every run separately records an outcome, evidence links, a bounded failure category, and a recommended next human or agent action. A failed or cancelled Run is reconciled into a visible Work item state instead of introducing a `failed` Work item state.

### 4. Multiple projects can move without becoming opaque

The Board is a portfolio control surface, not a single-project queue. One autopilot can select work across several projects while respecting a global concurrency cap, a per-project cap, project-specific approval gates, and fair scheduling. The person can still see which projects are healthy, blocked, idle, or consuming execution capacity.

Gareji Board, Gareji Core, and every individual plugin are separate products and therefore separate Board projects. `Gareji Plugins` may be used as a portfolio category, but it is not a single project that owns all plugin work.

### 5. Knowledge tools are sources, not the product boundary

Obsidian, Zettelkasten, GBrain, and LLMWiki are not interchangeable product boundaries. A **Markdown Zettelkasten** is the first real adapter; the Board reads a configured project directory (for example, the user's `4_Project` convention) and derives project context from its children. The folder convention belongs to the connected workspace, not to Zettelkasten itself. Obsidian is only one optional editor for that Markdown.

The Board connects to a **Knowledge workspace** through a **Knowledge Adapter** and imports explicit project and work-item references from it. A workspace can contain many projects, and a project can enable several Knowledge connections at once. Context is read across all enabled read connections with source identity preserved. The same Progress Checkpoint is projected independently to every enabled write connection. Adding, removing, or combining local JSON, Markdown Zettelkasten, GBrain, or LLMWiki does not change Board projects, Work items, agent profiles, or Progress Checkpoints.

### 6. Direct development remains visible

A person may enter a connected project and work without Gareji Runner. A manual command or trusted lifecycle Hook records the same Progress Checkpoint used by Runner results, so the Board timeline and knowledge-workspace progress notes converge without requiring duplicate status entry.

## Authority split

Each fact has one authoritative owner:

| Fact | Authority |
|---|---|
| Purpose, specifications, decisions, and durable background | Knowledge workspace |
| Work-item state, priority, dependencies, agent assignment, and autopilot policy | Gareji Board |
| Source code, Git history, and test results | Execution workspace |
| Full execution evidence and raw logs | Gareji Core / Runner sidecars |
| Evidence summary, failure category, and links to raw evidence | Gareji Board |
| Progress Checkpoint ledger, outbox, and delivery state | Gareji Core Progress Recorder local state |
| Generated progress notes | Projections from one Progress Checkpoint into enabled Knowledge connections |
| Agent instructions and Skills | Versioned agent files referenced by the Board profile |

An adapter may display or propose updates to another authority, but it does not silently become a second writable owner. The first Markdown adapter therefore imports context and proposes knowledge-workspace write-back for human review; it does not use Markdown task fields as a competing live state store.

## Minimum domain model

| Object | Essential fields |
|---|---|
| Knowledge workspace | adapter type, local location, sync status, source identity |
| Knowledge connection | project, workspace, read policy, progress-write policy, required/optional delivery |
| Execution workspace | local location, repository identity, connection status, write policy |
| Project | name, health, execution cap, approval policy |
| Context source | workspace, source path or URL, freshness, read/write policy |
| Work item | project, title, context references, state, priority, approval requirement |
| Agent profile | role, capabilities, limits, optional model override, Codex profile, trusted plugin reference |
| Autopilot policy | selection order, concurrency cap, allowed states, approval gates |
| Run | selected reason, resolved model and source, start/end, outcome, evidence, failure category |
| Handoff | what changed, what to review, recommended next action |
| Progress Checkpoint | source, actor, project, optional work item, outcome, summary, changed paths, verification, evidence, recommended state |
| Checkpoint delivery | checkpoint, destination, status, attempts, last error |

## Initial scope

- Local board data and bundled demo fixture.
- A one-action disposable sample launch with bundled Skills.
- In-place connection of an existing local repository or directory without moving or copying it.
- Deterministic one-item selector.
- Gareji Safe Autopilot Work item states, Todo Runner reconciliation, stop decisions, fast exits, and candidate-skip reasons.
- Multi-project portfolio selection with global and per-project execution caps.
- Four example agent profiles backed by bundled, inspectable Skills.
- Workspace-default, per-agent, and one-run model selection with visible resolution order.
- Run timeline and failure-state presentation.
- Markdown Zettelkasten project-directory import as the first real adapter, with a configurable project-directory path.
- A bundled `local_json` Knowledge Adapter so the hackathon demo proves adapter replacement without an external account.
- Simultaneous context reads and independent Progress Checkpoint projection across multiple enabled Knowledge connections.
- Separate mapping from each Board project to the local repository or directory in which its agent may work.
- Markdown context import/export for the Markdown Zettelkasten.
- A Knowledge Adapter contract that can later support GBrain brains and LLMWiki repositories without changing Board semantics.
- Gareji Core invocation through a fixed, validated integration boundary.
- One Progress Recorder shared by Runner, MCP, `gareji checkpoint`, and a trusted Codex `Stop` Hook.
- A durable local checkpoint outbox with idempotent retry and visible `pending`, `synced`, `conflict`, and `failed` delivery states.
- Board timeline and append-only Markdown progress-note projections from the same Progress Checkpoint.
- An Activity Inbox for checkpoints that have a project but no explicit Work item.
- Core-backed SQLite application-data storage for active Work item references, checkpoint ledger, outbox, and delivery receipts; no daemon required in v0.
- A small Gareji MCP interface for project discovery, context retrieval, active Work item selection, progress recording, and delivery-status inspection.

## Explicitly out of scope

- Multi-tenant collaboration, billing, marketplaces, or arbitrary remote plugin installation.
- Arbitrary code execution from a board card.
- Building a general-purpose remote orchestration platform.
- Always-on autonomy before the one-shot workflow is trustworthy.
- Continuous file watching, raw transcript synchronization, or full-diff copies in progress notes.
- Automatic commit or push of a documentation repository without a separate user policy and approval.

## Future production adapters

Future production adapters may supply richer task state or invoke additional agent runtimes. Markdown Zettelkasten, GBrain, and LLMWiki are Knowledge Adapters; Obsidian is an optional editor. The Board must remain comprehensible without any of them. If a Board workflow cannot run from the fixture, it is not part of the hackathon demo.
