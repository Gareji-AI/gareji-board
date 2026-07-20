# Hackathon demo story

The demo proves two product claims in one local workflow: direct development stays visible without Gareji Runner, and several Knowledge Adapters can be used together without changing project or task semantics.

## Act 1: Start without setup

1. Run `gareji-board demo --reset` (or `cargo run -p gareji-board-bridge --bin gareji-board -- demo --reset` from the source checkout).
2. Open the three bundled product projects and inspect the visible task states.
3. Show the dependency-free `local_json` and Markdown Zettelkasten Knowledge connections enabled together, plus the bundled Skills.

## Act 2: Work outside Runner

1. Select `BOARD-1` and mark it as the active Work item.
2. Enter its disposable Execution workspace and make a bounded change directly with Codex.
3. End the turn without starting Gareji Runner.
4. The trusted `Stop` Hook records one Progress Checkpoint in the local outbox.
5. Reopen or focus Gareji Board and show the same checkpoint on the task timeline.

## Act 3: Project into connected knowledge

1. Project the same checkpoint through both `local_json` and Markdown Zettelkasten.
2. Show one delivery receipt per destination and `fully synced` when both succeed.
3. Disconnect the Markdown workspace, record another checkpoint, and show JSON as `synced` while Markdown is `pending`.
4. Reconnect Markdown and show its existing delivery become `synced` without duplicating either projection.

## Judge-visible result

The Board project, Work item ID, agent profile, checkpoint payload, and task history remain unchanged while knowledge connections are combined or temporarily unavailable. The user sees what happened, which destinations are current, what is pending, and what requires human judgment.
