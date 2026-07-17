---
name: review-work-item
description: Review an execution-workspace change against its selected work item, acceptance criteria, repository instructions, and available evidence. Use when a work item reaches in_review, when verifying an agent result, or when deciding whether work is done, blocked, or needs another implementation pass.
---

# Review Work Item

Review through the supplied work item and execution workspace. Do not expand into unrelated repository cleanup.

## Workflow

1. Read the acceptance criteria and applicable workspace instructions.
2. Inspect the relevant change and verification evidence.
3. Report actionable findings first, ordered by severity, with precise file or evidence references.
4. Distinguish correctness problems from optional improvements.
5. Return one recommendation: `done`, `in_review`, or `blocked`, with the reason and next action.

Do not implement fixes unless the selected work item explicitly authorizes implementation. Do not approve work whose claimed checks or evidence cannot be verified.
