# Gareji Board

Gareji Board is a no-code workboard for human–AI teams. It makes autonomous work visible: what is ready, which custom agent is running it, what evidence it produced, and where it stopped.

## Product promise

People should not need to write automation code just to try an AI operating loop. Create work items, choose an agent profile, turn on an autopilot policy, and inspect the resulting evidence in one place.

Gareji Board is the reference product for [Gareji Core](https://github.com/Gareji-AI/gareji-core). The Board owns its user experience and local work state; Core owns trusted plugin execution, compact results, and audit sidecars.

## First demo

The initial demo must work with bundled sample data and no account, API key, Obsidian installation, or external orchestration service. It demonstrates:

1. A board using the Gareji work-item states `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, and `cancelled`.
2. A user-selected custom agent profile for each eligible item, with a separately selectable Codex model.
3. An autopilot that deterministically selects one safe next item using Gareji Safe Autopilot stop, fast-exit, reconciliation, and candidate-skip semantics.
4. An evidence-first run record showing the decision, result, failure category when relevant, and the next human action.
5. A portfolio view that coordinates several projects without losing each project's state, limits, or approval gates.

The first real Knowledge Adapter is a **Markdown Zettelkasten**. The Board reads a configured project directory (for example, the user's `4_Project` convention) and treats its children as project context. Zettelkasten itself does not require this folder layout. Obsidian remains an optional editor for that Markdown, not the integration boundary or a required installation.

The bundled demo uses a separate Markdown workspace under `examples/demo-workspace`. It represents Gareji Board, Gareji Core, and the Zettelkasten plugin as three independent product projects without copying personal notes or source repositories into the demo. Each additional plugin becomes its own product project and can be connected to its actual local repository as an execution workspace.

## Planned first launch

The first launcher exposes two choices:

- **Open sample** creates a resettable, disposable session from the bundled workspace and sample Skills. It never writes into the bundled fixture itself.
- **Add existing project** connects a selected local repository or directory in place. Gareji Board stores a reference and permission policy; it does not move or copy the project.

The target command for the sample path is `gareji-board demo`. See [the architecture](docs/architecture.md) and [the onboarding design](docs/onboarding-design.md) for responsibility, connection, and trust rules.

## Runner and direct work

Work remains visible whether it was started by Gareji Runner or performed directly inside a connected project. Runner results, a manual `gareji checkpoint`, and a trusted Codex `Stop` Hook all emit the same immutable Progress Checkpoint. Gareji Board records it first, then projects an append-only progress note into every enabled write connection without turning those notes into additional task-state owners.

Knowledge Adapters are composable as well as replaceable. The bundled demo reads from and projects progress to both Markdown Zettelkasten and a dependency-free local JSON adapter at the same time; future GBrain and LLMWiki adapters keep the same Board and Checkpoint semantics.

See [the Progress Checkpoint design](docs/progress-checkpoint-design.md), [the Gareji MCP design](docs/mcp-design.md), and [the example event](examples/progress-checkpoint.json).

Gareji uses SQLite-backed state under the operating system's application-data location. Board-owned coordination records and Core-owned operational records remain behind separate Module Interfaces even if the implementation shares one database. JSON remains a fixture, import/export format, and demo Knowledge Adapter rather than the live control store.

GBrain and LLMWiki are future adapters: useful real-world integrations, not prerequisites for understanding or using the Board. One connected knowledge workspace may expose zero, one, or many projects; the Board keeps that source relationship visible instead of assuming that one app equals one project.

## Boundaries

- Do not turn the Board into an opaque general-purpose orchestrator.
- Do not let an autopilot execute arbitrary shell commands.
- Keep credentials and raw audit sidecars local and out of board data.
- Require explicit human approval for public, secret-bearing, destructive, or production-changing actions.
- Keep the first experience runnable from one command with bundled fixtures.

## Status

Private local development repository. No public repository, publication, or release has been created.

See [the product brief](docs/product-brief.md), [the onboarding design](docs/onboarding-design.md), [the control semantics](docs/control-semantics.md), [the Runner design](docs/runner-design.md), and [the demo fixture](examples/demo-board.json).

Run the current Rust desktop shell with `cargo run -p gareji-board-app`. It creates disposable sample coordination state in the operating system's local application-data directory and loads recent Progress Checkpoints through a local `gareji-core bridge` when Core is installed. Project-only Checkpoints appear in the Activity Inbox and can be attached to an existing Work item or used to create and attach a new `todo` Work item without rewriting Core history. Linked state recommendations can then be accepted or dismissed from the Activity timeline; accepted supported recommendations update Board state transactionally. The Work item control surface groups individual items into canonical-state Kanban lanes and applies explicit human state changes without letting a stale screen overwrite newer Board state. `GAREJI_BOARD_DB`, `GAREJI_CORE_BIN`, and `GAREJI_CORE_DB` can select explicit local state and Core installations. `cargo run -p gareji-board-bridge --bin gareji-board -- seed-sample` initializes the same sample explicitly for Core integration; the local Board Bridge validates active-work selection, while Runner execution is not wired yet.

Validate code, documentation, schemas, references, and public-data hygiene with `cargo test --workspace` and `cargo xtask validate`. Contribution and security expectations are documented in [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).
