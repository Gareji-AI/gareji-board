# Attach project-only Checkpoints in Board

Gareji Board stores one final Checkpoint attachment for a project-only Progress Checkpoint instead of rewriting the Core-owned Checkpoint. The attachment must target a Work item in the same Board project, either one that already exists or one created atomically with the attachment. A newly created Work item begins in `todo`, while the Checkpoint recommendation remains a separate human decision. Identical retries are idempotent, while a different later target is rejected so reconciliation and displayed history retain one stable identity.
