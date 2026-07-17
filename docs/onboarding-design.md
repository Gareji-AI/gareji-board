# Sample and existing-project onboarding

Gareji Board has two first-launch paths. Both produce the same Board project model, agent profiles, policy controls, and run history after onboarding.

## Open sample

`gareji-board demo` creates a disposable session from the bundled fixture, then opens the Board. The session includes three sample projects, four agent profiles, four inspectable instruction files, four inspectable Skills, safe autopilot settings, and example success and blocked outcomes.

The launcher copies writable demo material into an application-owned session directory. Knowledge notes come from the demo Knowledge workspace; Agent instructions and Skills come from the separate demo Execution workspace fixture. It never writes into either bundled fixture, a personal knowledge workspace, or an existing source repository. The user can reset the session to its original state.

## Connect an existing execution workspace

For an existing Board project, the user selects an existing local repository or directory. Gareji Board resolves the directory and stores an in-place Execution workspace connection; it does not relocate, copy, initialize, or publish the project. The v0 Board stores at most one selected Execution workspace connection per Board project. Replacing it is an explicit local configuration change, not a source-code operation.

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

## Skill trust

Bundled Skills are inspectable and enabled in the sample. Project-local Skills may be discovered at `.agents/skills/<skill-id>/SKILL.md` under the connected Execution workspace but remain disabled until the user explicitly enables them for that project. Gareji Board does not download or install remote Skills during onboarding.

Agent profiles reference Skills by stable ID. Instructions, Skills, runtime, and model remain separate choices so a user can change the model without redefining the agent.

## Codex integration enablement

The Codex integration is opt-in. The user installs and enables its MCP configuration, Skills, and lifecycle Hook, then reviews and trusts the Hook before it can run. A changed Hook requires review again.

This is integration-level trust, not a request to copy Hook files into every repository. For each connected project, the user separately chooses read-only or workspace-write access and whether Progress Checkpoints may be projected into a Knowledge workspace.

The integration exits without recording when the current directory is not inside a connected Execution workspace. Disabling the integration stops automatic capture while preserving manual `gareji checkpoint` and previously stored events.

## Workspace Onboarding module

The module exposes two operations:

```text
open_sample() -> Board session
connect_execution_workspace(project_id, path) -> Execution workspace connection
```

Fixture materialization, path validation, repository detection, instruction discovery, Skill discovery, identity generation, and connection diagnostics remain inside the module. Callers receive a validated session or connection plus bounded diagnostics; they do not reproduce onboarding rules. Access-policy selection is deferred until Runner wiring, because connecting a directory alone must not grant execution authority.
