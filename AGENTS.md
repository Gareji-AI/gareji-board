# Gareji Board contributor guide

## Scope

Gareji Board owns the human control surface, Work item state, portfolio coordination, approvals, and visible delivery status. Gareji Core owns generic trusted execution and progress intake. Runner, MCP, and Knowledge Adapter implementations must remain replaceable through their documented Interfaces.

Do not move raw execution logs, credentials, arbitrary shell commands, provider-specific runtime fields, or knowledge-provider task state into Board data.

## Domain rules

- Read `CONTEXT.md` and relevant ADRs before changing terminology or state behavior.
- Gareji Board is authoritative for Work item state.
- A failed Run is an outcome, not a Work item state.
- Progress Checkpoints are immutable and recommend state; they do not apply it directly.
- Delivery is tracked independently for every enabled Knowledge connection.
- Direct work uses the connected checkout; mutating Runner work uses an isolated Git worktree.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask validate
```

## Change discipline

- One commit represents one independently reviewable and revertible meaning.
- Keep schemas, examples, tests, and documentation with the behavior they prove.
- Use an English Conventional Commit subject.
- Never include private strategy, pricing, provenance investigations, credentials, personal paths, customer data, or raw transcripts in public history.
