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

## Active Work assessment

Before Core stores an active Work item reference, Board assesses the explicit project and Work item pair without changing either record. `todo`, `in_progress`, and `in_review` are eligible. `backlog` is not admitted, `blocked` cannot proceed, and `done` or `cancelled` is terminal. Unknown and cross-project identities are rejected as not found.

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

## Checkpoint recommendation review

Project-only Progress Checkpoints remain in the Activity Inbox until a person attaches one to a Work item in the same Board project. The target may be an existing Work item or a new `todo` Work item created atomically with the attachment. Board records the attachment without changing the Core-owned Checkpoint or applying its state recommendation. An identical retry succeeds without creating another record; selecting a different Work item after attachment is rejected so later reconciliation uses one stable Work item identity.

A linked Progress Checkpoint may recommend a Work item state, but Board does not apply it during intake. The Activity timeline offers an explicit accept or dismiss decision. Accepting `in_progress`, `in_review`, `blocked`, or `done` updates the linked non-terminal Work item and records the decision atomically; accepting the current state is an idempotent no-change decision. Dismissing records the judgment without changing state. Recommendations to `backlog`, `todo`, or `cancelled` require a separate explicit Work item action because they represent admission, retry, or termination policy rather than progress reconciliation.

Each Checkpoint receives at most one reconciliation decision. A later correction changes the Work item through its own explicit action and does not rewrite the historical judgment or the immutable Checkpoint.

## Explicit human transitions

The Work item control surface lets a person explicitly select any canonical Work item state. The request carries the state the person observed; Board rejects the change when the stored state has moved since that observation instead of overwriting newer coordination. Selecting the already-stored state is an idempotent no-op. A human transition changes only the Board-owned Work item and does not rewrite a Checkpoint, attachment, reconciliation decision, or Run outcome.

## Explicit Agent plan updates

The Work item control surface lets a person atomically change the assigned Agent profile and required Agent capabilities without changing Work item state. The request carries the complete Agent plan the person observed; Board rejects the update when either the stored assignment or capability requirements changed first instead of overwriting newer coordination. Saving an identical plan is an idempotent no-op.

An Agent plan may deliberately be unassigned or require a capability that no current Agent profile declares. Such a plan is valid scheduling intent, but Safe Autopilot skips the Work item until a suitable profile is assigned. A referenced Agent profile must exist when the update is applied. Agent plan updates do not create a Run, select a model, or grant Core capability.

## Explicit Agent profile management

The Agent catalog lets a person create a Board-owned Agent profile or atomically replace an existing profile's role, declared Agent capabilities, optional Agent instruction reference, and Skill references. A profile ID is a stable lowercase identifier and cannot be renamed after creation. The v0 control surface does not delete profiles because Work items may still refer to them.

An edit request carries the complete Agent profile the person observed; Board rejects the change when the stored role, capabilities, or references changed first instead of overwriting newer coordination. Saving an identical existing profile is an idempotent no-op. A profile with no declared capabilities is valid, but it cannot satisfy a Work item with capability requirements. An Agent instruction reference is optional, and a profile may have no Skill references.

An Agent instruction reference is a forward-slash relative path from an Execution workspace to one versioned instruction file. Absolute paths, empty path segments, `.` segments, `..` traversal, backslashes, and control characters are rejected. Skill references use the same stable lowercase identifier form as Agent profiles and are stored in stable lexical order without duplicates.

Board validates the shape of these references but does not copy instruction contents, install Skills, or require every reference to resolve while the profile is being edited. This keeps a profile portable across projects. A future Runner preflight must resolve the instruction file and every enabled Skill for the selected Execution workspace and fail closed before starting when it cannot. The current read-only candidate preview remains a scheduling assessment rather than that execution preflight.

Changing a profile does not rewrite any Work item's Agent plan. Because Safe Autopilot evaluates the current catalog, removing a declared capability can immediately make an assigned Work item ineligible until a person repairs the profile or Work item. Profile management does not create a Run, select a model, grant Core capability, or treat instruction and Skill references as trusted execution authority.

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
- an explicit Approval requirement has no trusted approval evidence;
- a dependency is not done-like;
- evidence preflight fails;
- another active item has the same scope;
- required agent capabilities are unavailable;
- a daily start cap, review cap, or drip-queue cooldown applies.

## Read-only candidate preview

The desktop Board may evaluate a bounded candidate preview before Runner execution is connected. This preview is explanatory only: it never creates a Run, changes Work item state, reserves capacity, records approval evidence, or grants execution authority. Cooldown, Core-capability, workspace, and evidence preflights remain required before a later execution path may start the selected item.

The v0 preview considers only canonical `todo` Work items whose Board project is below its execution cap, whose persisted Approval requirement is `none`, whose persisted Work item dependencies are done-like, and whose assigned Agent profile declares every required Agent capability. A dependency on canonical `done` or `cancelled` is done-like; supported import aliases are normalized before evaluation. An explicit Approval requirement is skipped until a later execution path can supply trusted, operation-bound approval evidence.

An unassigned Work item is skipped because Runner has no intended Agent profile. When an assigned profile lacks several requirements, the preview reports the first missing Agent capability in stable lexical order. These Board scheduling claims do not replace Core capability policy or trusted approval evidence. An assignment referring to a missing Agent profile fails closed as corrupt coordination input.

Runnable candidates are ranked deterministically by:

1. lowest project execution-cap utilization, compared as exact fractions;
2. lowest numeric Work item priority;
3. project ID;
4. Work item ID.

The preview reports state-ineligible, project-capacity, approval-required, first-unresolved-dependency, unassigned, missing-Agent-capability, and lower-ranked candidates separately so the person can understand the result. Reaching the configured global concurrency cap or finding no runnable candidate returns `decision=continue`, no candidate, and `fast_exit_required=true`; it is not a failure. A zero global cap, a duplicate Work item or Agent profile identity, a Work item whose project is missing from the portfolio, a dependency whose Work item is missing, or an assignment whose Agent profile is missing fails closed with `decision=stop`, no candidate, and `fast_exit_required=true`.

Work items created from the Activity Inbox receive priority `100` and no Agent assignment until a person records an Agent plan. The bundled desktop sample uses a global preview concurrency cap of `2`.

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
| Desktop preview global concurrent Runs | 2 |

Later products may expose custom policy profiles. The `gareji_safe_autopilot_v0` profile keeps these semantics stable so customization cannot silently change the default safety behavior.
