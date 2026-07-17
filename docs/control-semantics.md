# Gareji Safe Autopilot control semantics

Gareji Board v0 defines a deterministic, evidence-first control policy for Backlog Promoter and Todo Runner. The policy is independently specified and runs without an external orchestration service.

## Work item states

The canonical states are:

| State | Meaning |
|---|---|
| `backlog` | Known work not yet admitted to the executable queue |
| `todo` | Admitted work eligible for Todo Runner selection |
| `in_progress` | Work with a confirmed active execution |
| `in_review` | Completed, failed, or interrupted work awaiting automated or human judgment |
| `blocked` | Work that cannot proceed until an explicit blocker is resolved |
| `done` | Verified completion or an explicit no-action result |
| `cancelled` | Work intentionally terminated without completion |

`completed`, `canceled`, and `closed` may be accepted as done-like import aliases for dependency resolution. Gareji Board persists only the canonical spellings above.

A Run has a separate lifecycle. `failed` and `error` are Run outcomes, not Work item states. A failed, errored, or cancelled assigned Run is normally reconciled to `in_review` so a human or later policy can choose rerun, `blocked`, or `cancelled`.

## Reconciliation before selection

Before selecting new `todo` work, the controller reconciles stale state:

- a `todo` item with an active Run becomes `in_progress`;
- a queued Run prevents a duplicate start;
- stale `in_progress` without an active Run returns to `todo`;
- a completed Run with a PR or review artifact becomes `in_review`;
- an explicit completed no-action result becomes `done`;
- a completed Run that still needs judgment becomes `in_review`;
- a failed, errored, or cancelled assigned Run becomes `in_review`;
- an evidence-preflight failure returns the item to `backlog` with a handoff;
- a completed `in_review` item with `pr_required=false`, no PR artifact, and no human-review requirement may become `done` automatically.

## Controller stop and fast exit

A controller-wide `decision=stop` means no candidate may start during that tick. The current compatibility rules stop when:

- the configured workspace sentinel or fixed-root preflight fails;
- another active controller occupies the same runtime lane during the same scheduling valve;
- the selector reports another explicit deterministic stop reason.

Backlog admission additionally stops when its queue caps are already reached: 30 AI-owned `todo` items or 20 PR-producing `todo` items.

`fast_exit_required=true` means the tick must end without extra inspection. It is also used when there is no candidate after reconciliation. Therefore, `no_candidate` is not a failure and does not change a Work item to `blocked` or `cancelled`.

The stop result contains no candidate and recommends `final_report_only`. Invalid, errored, unsupported, or truncated selector input fails closed and starts nothing.

## Candidate-level skips

These conditions skip one candidate while allowing the controller to consider another:

- human-owned work;
- an active or queued Run already exists;
- cooldown is active;
- `waiting_on`, `blocked_reason`, or blocked wording is present;
- a dependency is not done-like;
- evidence preflight fails;
- another active item has the same scope;
- required agent capabilities are unavailable;
- a daily start cap, review cap, or drip-queue cooldown applies.

## Compatibility defaults

| Limit | Value |
|---|---:|
| Backlog Promoter AI `todo` cap | 30 |
| Backlog Promoter PR-producing `todo` cap | 20 |
| Backlog Promoter review-fix exempt cap | 10 |
| Normal promotions per tick | 5 |
| Special promotions per tick | 8 |
| Todo Runner normal PR starts per day | 10 |
| Todo Runner review-fix starts per day | 10 |
| Review soft / hard / stop caps | 15 / 25 / 35 |
| Todo Runner starts per tick | 1 |

Later products may expose custom policy profiles. The `gareji_safe_autopilot_v0` profile keeps these semantics stable so customization cannot silently change the default safety behavior.
