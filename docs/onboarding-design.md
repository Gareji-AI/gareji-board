# Sample and existing-project onboarding

Gareji Board has two first-launch paths. Both produce the same Board project model, agent profiles, policy controls, and run history after onboarding.

## Open sample

`gareji-board demo` creates a disposable session from the bundled fixture, then opens the Board. The session includes three sample projects, four agent profiles, four inspectable instruction files, four inspectable Skills, safe autopilot settings, and example success and blocked outcomes.

The launcher copies writable demo material into an application-owned session directory. Knowledge notes come from the demo Knowledge workspace; Agent instructions and Skills come from the separate demo Execution workspace fixture. It never writes into either bundled fixture, a personal knowledge workspace, or an existing source repository. The user can reset the session to its original state.

## Add or connect an existing execution workspace

The user may add an existing local repository or directory as a new Board project, or select one for an existing Board project. Adding creates an `idle` Board project with an explicitly supplied stable ID, display name, and positive execution capacity, together with its first local Execution workspace connection in one local operation. It does not derive a Board identity from source files, repository metadata, or a knowledge workspace.

Gareji Board resolves the selected directory and stores an in-place Execution workspace connection; it does not relocate, copy, initialize, or publish the project. The v0 Board stores at most one selected Execution workspace connection per Board project. Replacing it is an explicit local configuration change, not a source-code operation.

The connection flow shows:

1. resolved local path and detected repository root, when present;
2. project name and stable local identity;
3. applicable workspace instructions;
4. discovered project-local Skills;
5. requested access: read-only or workspace-write;
6. optional links to one or more knowledge context sources;
7. actions that still require human approval.

A non-Git directory is valid. A missing, unreadable, or moved directory remains visible as disconnected instead of being silently removed. Board stores the canonical local path needed to find the workspace again, but does not store its source files, Git state, instruction contents, Skill contents, or access grants.

The project card performs a fresh, read-only connection check when it is displayed. It reports whether the selected local directory is available and, when an ancestor contains a `.git` file or directory, the nearest Git repository root. The check does not read source contents, inspect branches or remotes, invoke Git, create a worktree, or change Runner eligibility; Runner repeats its own authoritative preflight immediately before a Run.

For the Codex-first v0, the project card also lists a root `AGENTS.md` when it resolves to a regular file below the canonical Execution workspace root. It reports the relative path only: Board does not read or copy the instructions, apply them to an Agent profile, or treat their presence as execution approval.

## Skill trust

Bundled Skills are inspectable and enabled in the sample. Project-local Skills may be discovered at `.agents/skills/<skill-id>/SKILL.md` under the connected Execution workspace but remain disabled until the user explicitly enables them for that project. Gareji Board does not download or install remote Skills during onboarding.

The project card lists only stable Skill IDs whose `SKILL.md` resolves to a regular file below the canonical Execution workspace root. Discovery reads directory and file identity only; it does not read Skill contents, follow a Skill that escapes through a symbolic link, install anything, or make a Skill runnable.

Agent profiles reference Skills by stable ID. Instructions, Skills, runtime, and model remain separate choices so a user can change the model without redefining the agent.

## Codex integration enablement

The Codex integration is opt-in. The user installs and enables its MCP configuration, Skills, and lifecycle Hook, then reviews and trusts the Hook before it can run. A changed Hook requires review again.

This is integration-level trust, not a request to copy Hook files into every repository. For each connected project, the user separately chooses read-only or workspace-write access and whether Progress Checkpoints may be projected into a Knowledge workspace.

The integration exits without recording when the current directory is not inside a connected Execution workspace. Disabling the integration stops automatic capture while preserving manual `gareji checkpoint` and previously stored events.

## Workspace Onboarding module

The module exposes three operations:

```text
open_sample() -> Board session
add_existing_project(project_id, name, execution_cap, path) -> Board project + Execution workspace connection
connect_execution_workspace(project_id, path) -> Execution workspace connection
```

Adding a project is atomic: a rejected or duplicate Board identity leaves no connection behind, and an unavailable directory leaves no project behind. Fixture materialization, path validation, repository detection, instruction discovery, Skill discovery, and connection diagnostics remain inside the module. Callers receive a validated session or connection plus bounded diagnostics; they do not reproduce onboarding rules. Access-policy selection is deferred until Runner wiring, because connecting a directory alone must not grant execution authority.
