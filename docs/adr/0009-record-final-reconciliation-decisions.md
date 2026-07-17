# Record final reconciliation decisions

Gareji Board records at most one final accept-or-dismiss decision for each Progress Checkpoint and applies an accepted state recommendation in the same local transaction. This keeps the visible decision and Work item state consistent, prevents repeated stale evidence from changing state later, and preserves Core's immutable checkpoint; correcting a mistaken decision uses a separate explicit Work item action rather than rewriting reconciliation history.
