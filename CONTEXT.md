# Gareji Board

Gareji Board is a control surface for human–AI work across projects and knowledge workspaces. This glossary keeps its integration language independent from any one note-taking application.

## Knowledge context

**Markdown Zettelkasten**:
A Markdown knowledge base organized using the Zettelkasten method. Its folder layout is a workspace convention rather than a requirement of Zettelkasten itself.
_Avoid_: Obsidian vault, note app

**Project directory**:
A configured directory inside a Markdown Zettelkasten whose direct children represent the user's projects, such as `4_Project`. It is a Gareji workspace convention and the discovery point from which the Board derives projects and their context.
_Avoid_: workspace, Board project list

**Knowledge workspace**:
A connected source of durable context, such as a Markdown Zettelkasten, GBrain brain, or LLMWiki repository. One workspace can contain many projects.
_Avoid_: project, application

**Knowledge Adapter**:
An Adapter that connects one kind of Knowledge workspace to the stable Board project and Progress Checkpoint model. Replacing it does not change Work item state, agent behavior, or execution history.
_Avoid_: model memory, note application, runtime

**Knowledge connection**:
A configured relationship between a Board project and one Knowledge workspace, including its context-read and progress-write policies. One Board project may have several enabled Knowledge connections at the same time.
_Avoid_: primary adapter, active backend

**Execution workspace**:
A local directory or repository in which an agent is permitted to perform project work. It is connected to a Board project but remains separate from its knowledge workspace.
_Avoid_: knowledge workspace, project directory

**Obsidian client**:
An optional Markdown editor that a person may use to view and edit a Markdown Zettelkasten. It is not a required Board integration or the source of truth.
_Avoid_: knowledge workspace, adapter

**Context source**:
A specific note, page, folder, or generated wiki resource that supplies context to a Board project or work item.
_Avoid_: project, task

## Work control

**Product**:
An independently developed and releasable part of the Gareji portfolio, such as Gareji Board, Gareji Core, or one specific plugin. Each plugin is its own product rather than a child of a shared plugin product.
_Avoid_: plugin collection, repository group

**Board project**:
A unit of coordinated work derived from a project directory or explicitly mapped from a connected knowledge workspace. In the Gareji portfolio, each product has its own Board project, execution workspaces, work items, health, capacity, and approval policy.
_Avoid_: note application, vault

**Work item state**:
The Gareji lifecycle classification of a unit of work: `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, or `cancelled`. A failed execution is a Run outcome, not a Work item state.
_Avoid_: Run state, failure category

**Work item transition**:
An explicit Board-owned change from one Work item state to another. A human transition is distinct from accepting a Checkpoint recommendation or reconciling a Run.
_Avoid_: checkpoint decision, Run outcome

**Work item dependency**:
A directed prerequisite from one Work item to another. The dependent item remains in its own lifecycle state while unresolved prerequisites make it ineligible for execution.
_Avoid_: blocker text, Run ordering

**Approval requirement**:
The Board-owned indication that starting a Work item needs an explicit human decision in addition to Core capability policy. It is not approval evidence and does not grant a capability.
_Avoid_: capability grant, automatic approval

**Agent profile**:
A stable Board-owned description of an agent role, its declared Agent capabilities, and references to its instructions and execution configuration. It is not a running agent or a model selection.
_Avoid_: agent process, model, prompt

**Agent capability**:
A Board scheduling claim that an Agent profile is suitable for a kind of work. It is distinct from a Core capability, which grants permission for a concrete execution operation.
_Avoid_: Core capability, Skill, permission

**Agent assignment**:
The Board-owned relationship selecting one Agent profile for a Work item. Assignment establishes intended responsibility but does not start a Run.
_Avoid_: Run, agent process

**Agent plan**:
The Board-owned combination of one Work item's Agent assignment and required Agent capabilities. Changing it updates scheduling intent atomically but does not start a Run or grant Core capability.
_Avoid_: agent configuration, Run configuration

**Active Work assessment**:
The Board-owned judgment that a specific Work item may be associated with current direct or Runner work. `todo`, `in_progress`, and `in_review` are eligible; unadmitted, blocked, and terminal work is not.
_Avoid_: Core permission, inferred branch match

**Gareji Safe Autopilot**:
The default deterministic control policy for safe selection, reconciliation, stopping, and fast exit. It is a stable built-in profile that future custom policies may replace explicitly.
_Avoid_: scheduler, agent, runtime

**Run state**:
The lifecycle of one execution attempt for a Work item. It remains separate from Work item state so a failed or cancelled Run can be reconciled into the appropriate visible work state.
_Avoid_: Work item state

**Autopilot decision**:
The controller-wide result for one tick: `continue` or `stop`. A skipped candidate does not by itself stop the controller.
_Avoid_: Work item state, Run outcome

**Fast exit**:
A controller instruction to end the current tick without further inspection or execution. It can accompany a hard stop or a successful tick with no candidate.
_Avoid_: failure, cancellation

**Authority boundary**:
The assignment of one authoritative owner to each kind of fact, with other copies treated as projections or links. Knowledge facts, control facts, source code, and execution evidence have different owners.
_Avoid_: synchronization, duplication

## Progress capture

**Progress Checkpoint**:
An immutable, evidence-linked record of meaningful progress from a Runner or direct human–AI work session. It may recommend a Work item state but never owns that state.
_Avoid_: task state, full log, autosave

**Progress Recorder**:
The single intake for Progress Checkpoints from Runner, MCP, lifecycle Hook, or manual command sources. Every source receives the same validation, deduplication, and delivery behavior.
_Avoid_: synchronizer, Hook, adapter

**Activity Inbox**:
A holding area for valid Progress Checkpoints that are linked to a Board project but not yet to a Work item. A human can attach them to an existing Work item or create one without losing the original record.
_Avoid_: backlog, failed sync

**Checkpoint attachment**:
A final Board-owned association between one project-only Progress Checkpoint and a Work item in the same Board project. The Work item may already exist or be created as part of the attachment; neither path modifies the immutable Checkpoint.
_Avoid_: checkpoint edit, inferred assignment

**Activity timeline**:
The Board's read-only chronological view of accepted Progress Checkpoints, state recommendations, and delivery results. It presents Core-owned progress evidence without becoming another checkpoint ledger or applying Work item transitions.
_Avoid_: task history, mutable log

**Reconciliation decision**:
The Board-owned, final judgment to accept or dismiss one Progress Checkpoint's state recommendation. Accepting may transition the linked Work item; dismissing preserves its current state, and neither action changes the Checkpoint.
_Avoid_: checkpoint edit, automatic transition

**Checkpoint delivery**:
The projection status of one Progress Checkpoint to a destination: `pending`, `synced`, `conflict`, or `failed`. Delivery state does not change the immutable checkpoint or the Work item state.
_Avoid_: Work item state, Run state
