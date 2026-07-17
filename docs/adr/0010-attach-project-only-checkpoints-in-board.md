# Attach project-only Checkpoints in Board

Gareji Board stores one final Checkpoint attachment for a project-only Progress Checkpoint instead of rewriting the Core-owned Checkpoint. The attachment must target an existing Work item in the same Board project; identical retries are idempotent, while a different later target is rejected so reconciliation and displayed history retain one stable identity.
