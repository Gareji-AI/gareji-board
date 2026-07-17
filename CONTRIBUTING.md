# Contributing to Gareji Board

Gareji Board is currently a private pre-release project. These rules prepare a public contribution surface without authorizing publication.

## Before changing behavior

1. Read [CONTEXT.md](CONTEXT.md), [the architecture](docs/architecture.md), and relevant ADRs.
2. Identify the authoritative owner of every fact being changed.
3. Keep external runtime and knowledge-provider details behind their Interfaces.
4. Update the schema and synthetic examples with the behavior they prove.

## Verification

```bash
python -m pip install -r requirements-dev.txt
python scripts/validate.py
```

## Commits

Use one coherent meaning per commit and an English Conventional Commit subject, such as `docs(checkpoint): define partial delivery status`. Include intent and verification in the body when they are not obvious.
