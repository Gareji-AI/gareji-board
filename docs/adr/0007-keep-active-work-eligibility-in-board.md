# ADR 0007: Keep active-work eligibility in Board

Status: accepted.

Gareji Board is the authority for Work item state, so it also owns the decision that a specific Work item may be associated with current direct or Runner work. The stable v0 rule accepts `todo`, `in_progress`, and `in_review`; it rejects `backlog` as not admitted, `blocked` as unable to proceed, and `done` or `cancelled` as terminal.

Board exposes one `assess_active_work` operation through a bounded local stdio Bridge Interface. It returns the attributed state and eligibility result without changing the Work item. Core calls this Interface before storing its operational active-work reference and fails closed when Board is unavailable.

This keeps state policy in one deep Board Module while allowing Core and Board to remain separately released products. Direct SQLite access from Core was rejected because it would couple Core to Board schema and duplicate authority. A mandatory daemon was rejected because one reusable child process satisfies the current local single-parent workflow.
