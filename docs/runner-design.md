# Gareji Runner design

## Purpose

Gareji Runner is the local execution module inside Gareji Board. It schedules eligible work, resolves an agent and model, invokes Codex through Gareji Core, and converts execution events into visible Board state.

The hackathon implementation supports Codex only. Board data identifies `runtime` and `model` separately, and the Runner accepts a runtime-neutral execution request. Codex-specific flags and event parsing remain inside the Codex implementation.

A generic adapter registry is deferred until a second runtime actually exists. This keeps the first implementation small without forcing Board, scheduling, or run-history semantics to change when another runtime is added later.

## Execution seam

The Runner's execution interface has one operation:

```text
execute(execution_workspace, work_item, agent_profile, runtime, model_override)
  -> execution events + final run result
```

For now, `runtime` must be `codex`. A future runtime supplies another adapter at this seam; it does not add provider-specific fields to work items or change the scheduler.

The execution workspace is the checked and explicitly connected local directory from which Gareji creates an isolated Run workspace. It is not the Markdown knowledge workspace. This lets a Board project use notes from `4_Project/Gareji-Board` while Codex works against the separate `gareji-board` repository.

## Run isolation

For a Git Execution workspace, every Runner-started Run uses its own Git worktree, including when the project's concurrency cap is one. Parallel Runs never share a worktree: each Work item receives a distinct branch and worktree registered against its Run. The connected checkout remains available for direct human–Codex work and keeps one explicit active Work item at a time.

The v0 Runner blocks a mutating Run for a non-Git Execution workspace because Git worktree isolation is unavailable. A non-Git directory may still be connected for context inspection and direct work. Gareji must not silently fall back to running an autonomous mutation in the shared directory.

Runner records the branch, worktree location, base revision, and final revision as Run evidence. Cleanup occurs only after the Progress Checkpoint and evidence are durable and the worktree has no unrecorded changes; otherwise the worktree remains visible for recovery.

## Model selection

Agent behavior and model choice are independent. A `researcher` keeps the same instructions and skills when its model changes.

Runner resolves the model in this order:

1. A one-run override selected immediately before execution.
2. The agent profile's optional model override.
3. The workspace's default model.
4. The user's Codex default when all Gareji values are `inherit`.

The hackathon demo sets the workspace default to `gpt-5.6`. The product interface still supports changing the default and choosing an override without editing instructions or skills.

## Agent profile

```yaml
id: researcher
role: Researcher
instructions: agents/researcher/AGENT.md
skills:
  - web-research
  - evidence-summary
capabilities:
  - research
  - evidence
codex_profile: safe-research
model: inherit
sandbox: workspace-write
timeout_minutes: 20
```

`codex_profile` selects technical Codex configuration such as sandbox, reasoning effort, and MCP setup. `model` only selects the execution model. Neither field defines the agent's role.

Agent capabilities are Board scheduling claims such as `research`, `implementation`, `review`, or `release`. A Work item declares the capabilities it needs and is assigned one Agent profile; Safe Autopilot skips it when the assignment is absent or the profile lacks a requirement. These claims do not grant filesystem, secret, publication, or production permissions. Core evaluates those execution capabilities separately for the concrete Run operation.

## Codex invocation

Runner invokes Codex with an argument array equivalent to:

```text
codex exec --profile <profile> --model <resolved-model> --json --output-schema <handoff-schema> <prompt>
```

Runner never builds this command through shell-string interpolation. An unknown or rejected model produces a bounded `configuration_error` before the work item starts.

## Run evidence

Every run records:

- runtime;
- requested model, if any;
- resolved model;
- whether it came from the run, agent, workspace, or Codex default;
- Codex profile;
- start and end time;
- compact result and JSONL evidence location;
- failure category and next action.

This makes model comparisons possible without making execution opaque.
