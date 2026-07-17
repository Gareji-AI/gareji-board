# Fan out progress to enabled Knowledge connections

One Board project may read from several Knowledge workspaces and project the same immutable Progress Checkpoint to every enabled progress-write connection. Gareji records delivery independently per destination and preserves partial success, choosing explicit multi-destination consistency over a single primary write backend so users can keep multiple knowledge systems current without duplicate status entry.
