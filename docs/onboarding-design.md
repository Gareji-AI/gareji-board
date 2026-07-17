# Sample and existing-project onboarding

Gareji Board has two first-launch paths. Both produce the same Board project model, agent profiles, policy controls, and run history after onboarding.

## Open sample

`gareji-board demo` creates a disposable session from the bundled fixture, then opens the Board. The session includes three sample projects, four agent profiles, four inspectable Skills, safe autopilot settings, and example success and blocked outcomes.

The launcher copies writable demo material into an application-owned session directory. It never writes into the bundled fixture, a personal knowledge workspace, or an existing source repository. The user can reset the session to its original state.

## Add existing project

The user selects an existing local repository or directory. Gareji Board validates it and stores an in-place connection; it does not relocate, copy, initialize, or publish the project.

The connection flow shows:

1. resolved local path and detected repository root, when present;
2. project name and stable local identity;
3. applicable workspace instructions;
4. discovered project-local Skills;
5. requested access: read-only or workspace-write;
6. optional links to one or more knowledge context sources;
7. actions that still require human approval.

A non-Git directory is valid. A missing, unreadable, or moved directory remains visible as disconnected instead of being silently removed.

## Skill trust

Bundled Skills are inspectable and enabled in the sample. Project-local Skills may be discovered under a configured skill directory but remain disabled until the user explicitly enables them for that project. Gareji Board does not download or install remote Skills during onboarding.

Agent profiles reference Skills by stable ID. Instructions, Skills, runtime, and model remain separate choices so a user can change the model without redefining the agent.

## Codex integration enablement

The Codex integration is opt-in. The user installs and enables its MCP configuration, Skills, and lifecycle Hook, then reviews and trusts the Hook before it can run. A changed Hook requires review again.

This is integration-level trust, not a request to copy Hook files into every repository. For each connected project, the user separately chooses read-only or workspace-write access and whether Progress Checkpoints may be projected into a Knowledge workspace.

The integration exits without recording when the current directory is not inside a connected Execution workspace. Disabling the integration stops automatic capture while preserving manual `gareji checkpoint` and previously stored events.

## Workspace Onboarding module

The module exposes two operations:

```text
open_sample() -> Board session
connect_existing_project(path, access_policy) -> Project connection
```

Fixture materialization, path validation, repository detection, instruction discovery, Skill discovery, identity generation, and connection diagnostics remain inside the module. Callers receive a validated session or connection plus bounded diagnostics; they do not reproduce onboarding rules.
