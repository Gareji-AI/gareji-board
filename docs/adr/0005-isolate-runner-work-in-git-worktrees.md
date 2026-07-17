# Isolate Runner work in Git worktrees

Every mutating Runner Run uses a dedicated Git worktree and records its branch and revisions as evidence, even when only one Run is active. This keeps autonomous work isolated from the user's connected checkout and makes parallel Runs recoverable; v0 blocks mutating Runner work in non-Git directories instead of silently falling back to a shared workspace.
