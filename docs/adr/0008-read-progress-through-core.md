# Read progress through Core

Gareji Board reads accepted Progress Checkpoints through the bounded Core Bridge instead of opening Core's SQLite tables or keeping a second writable ledger. This preserves Core as the checkpoint authority and allows its storage to evolve, while Board converts the returned records into an Activity timeline and retains sole authority over reconciliation and Work item transitions.
