---
name: implement-bounded-work-item
description: Implement one approved and bounded work item inside its connected execution workspace, preserving unrelated changes and verifying the result. Use for scoped feature, bug-fix, test, or documentation changes that allow workspace writes and do not require publication, destructive actions, secrets, or production changes.
---

# Implement Bounded Work Item

Work only inside the execution workspace supplied by Gareji Board and only on the selected work item.

## Workflow

1. Read the work item, acceptance criteria, supplied context sources, and applicable workspace instructions.
2. Inspect existing changes before editing. Preserve unrelated user work.
3. Stop and return `blocked` when the task is ambiguous, exceeds its approval policy, or requires access outside the connected workspace.
4. Make the smallest cohesive change that satisfies the acceptance criteria.
5. Run the narrowest relevant verification, then broader checks when proportionate to risk.
6. Return changed files, checks performed, result, evidence references, remaining risks, and the recommended next state.

Never publish, push, merge, release, deploy, expose secrets, or perform destructive cleanup. Recommend `in_review` after a material change; recommend `done` only for an explicitly verified no-action or fully complete result.
