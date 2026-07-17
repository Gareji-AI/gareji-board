---
name: write-project-handoff
description: Produce a concise, evidence-linked handoff for one Gareji work item. Use after implementation, review, investigation, no-action, failure, or blocking outcomes to explain what changed, what was verified, what remains uncertain, and what a human or agent should do next.
---

# Write Project Handoff

Write from supplied run results and evidence. Do not claim changes, tests, or approvals that are not present in the evidence.

## Output

Return these fields:

- `outcome`: completed, needs_review, blocked, failed, or no_action;
- `summary`: the result in no more than three sentences;
- `changed`: changed artifacts or `none`;
- `verified`: checks and their results or `not_verified`;
- `evidence`: compact paths or run references;
- `risks`: unresolved risks or `none`;
- `next_action`: one concrete human or agent action;
- `recommended_state`: `in_review`, `done`, or `blocked`.

Keep full logs outside the handoff. Link to evidence instead of copying it into the response.
