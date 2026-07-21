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

**Execution workspace connection**:
The Board-owned local association between one Board project and its currently selected Execution workspace. It records only the connection kind and local location; source files, Git state, access grants, and execution evidence remain outside Board coordination state. In v0, a project has at most one selected connection.
_Avoid_: Runner workspace, repository mirror, access grant

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
A unit of coordinated work derived from a project directory, explicitly mapped from a connected knowledge workspace, or explicitly added from an existing Execution workspace. In the Gareji portfolio, each product has its own Board project, execution workspaces, work items, health, capacity, and approval policy.
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

**Agent instruction reference**:
An optional portable path from an Execution workspace to a versioned file that defines an Agent profile's behavior. It identifies instructions without copying their contents into Board data.
_Avoid_: prompt text, model configuration, absolute path

**Skill reference**:
A stable identifier associating an Agent profile with a versioned Skill that may be resolved for an Execution workspace. It neither installs the Skill nor grants execution permission.
_Avoid_: Agent capability, Core capability, Skill contents

**Agent behavior inspection**:
A Board-owned, read-only assessment of whether an Agent profile's instruction and Skill references are present inside one selected Execution workspace. It does not read their contents, enable a Skill, establish trust, or replace Runner preflight.
_Avoid_: execution preflight, Skill trust, instruction loading

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

**Control graph**:
A Board-owned, versioned description of the allowed control flow among focused Agent loops, deterministic gates, audits, approvals, and terminal outcomes. It governs organization and routing without replacing Work item dependencies or Core capability policy.
_Avoid_: Work item graph, runtime workflow, prompt chain

**Orchestration Blueprint**:
A project-independent, immutable graph describing a reusable way to organize approaches, gates, audits, approvals, and terminal outcomes. It declares required facts and capabilities without naming a concrete Board project, Agent profile, or Execution workspace.
_Avoid_: Project workflow, fixed project graph, prompt chain

**Approach Note**:
A Markdown knowledge artifact describing one reusable way to perform work, including its applicability, required capabilities, typed inputs, expected outputs, risk, and evidence requirements.
_Avoid_: prompt, Agent profile, Work item

**Note Socket**:
A typed connection point through which an Approach Note receives context or work and produces evidence, artifacts, signals, or approval requests inside an Orchestration Blueprint.
_Avoid_: arbitrary attachment, untyped edge, file copy

**Control node**:
One named stage in a Control graph. A node may invoke an Agent profile or represent a deterministic gate, audit, approval boundary, or terminal outcome; it is a description of allowed control, not a running agent.
_Avoid_: agent process, Work item, Run

**Control route**:
A named, typed edge from one Control node to another that may be selected only for its declared signal and policy conditions. A route describes allowed movement; selecting it does not itself start a Run or grant a capability.
_Avoid_: Work item dependency, arbitrary next prompt, Core permission

**Graph revision**:
An immutable version of one Control graph. Project configuration and in-flight route decisions refer to an exact revision so later edits cannot silently change active work.
_Avoid_: mutable graph, current topology

**Graph entry**:
A named permitted starting Control node in one Graph revision. A graph may expose several entries for different work shapes, so `root` is reserved for the human-owned root goal rather than used as an entry synonym.
_Avoid_: graph root, first Work item

**Project graph binding**:
The Board-owned selection of a Graph revision and Graph entry for one Board project. It establishes the project's default organization without assigning a Work item or starting a Run.
_Avoid_: Agent plan, Runner configuration, Work item dependency

**Blueprint Application**:
An inspectable proposal or accepted intent to use one exact Orchestration Blueprint revision and entry for a concrete Board project and Work item candidate. It does not itself select an Agent profile, start a Runner, or grant a capability.
_Avoid_: Project graph binding, Run, automatic graph mutation

**Runtime Binding**:
The immutable execution-time resolution of one Blueprint Application to a concrete Board project, Work item, Agent profiles, Approach Note fingerprints, and Execution workspace.
_Avoid_: Orchestration Blueprint, mutable project configuration, Core capability grant

**Portfolio orchestration graph**:
A Board-owned, versioned description of how coordination moves among Project Selectors, Blueprint Applications, and bounded post-actions. Concrete projects and execution targets remain unresolved until an accepted application creates a Runtime Binding.
_Avoid_: global Control graph, project-internal workflow, remote scheduler

**Portfolio orchestration node**:
One stage in a Portfolio orchestration graph: a Project Selector, a bounded post-action, or a terminal outcome. It describes coordination intent and does not itself start a Runner.
_Avoid_: Control node, Work item, agent process

**Project Selector**:
A Portfolio orchestration node that deterministically finds eligible work across all or an explicit subset of managed Board projects without fixing one project into the graph.
_Avoid_: fixed Project Invocation, repository scan, direct Runner request

**Portfolio schedule**:
The manual or interval cadence configured for a Portfolio orchestration graph. A schedule is inert configuration until a local scheduler creates a Portfolio Run.
_Avoid_: running timer, Run state, Core authority

**Portfolio schedule control**:
The runtime decision to permit or pause automatic ticks for one immutable Portfolio schedule. Pausing it preserves the current Portfolio Run and does not block an explicit manual tick.
_Avoid_: Run pause, graph revision, scheduler configuration

**Portfolio Run**:
One durable execution of a Portfolio orchestration graph, advancing at most one bounded node per scheduled tick unless an explicitly approved policy says otherwise.
_Avoid_: Runner Run, Control graph position, hidden background loop

**Route decision**:
An immutable Board-owned record of the current Control node, observed signal, selected Control route, policy result, evidence references, and resulting next node. A model may propose a route, but deterministic Board policy accepts or rejects it.
_Avoid_: model thought, Run outcome, Checkpoint

**Work item graph position**:
The Board-owned pin of one eligible Work item to an exact Graph revision, entry, and current Control node. It is created before the first Agent Loop execution and advances only with an accepted Route decision, so later Project graph binding changes cannot redirect in-flight work.
_Avoid_: Work item state, Run state, Project graph binding

**Agent Loop execution target**:
The concrete Agent profile identity resolved from one Work item's current Agent Loop Control node. It is scheduling input for constructing a Runner request and does not start a Run, trust Agent behavior, or grant a Core capability.
_Avoid_: Run, agent process, Core permission

**Runner graph completion**:
The Board controller operation that first records a started Runner result as a Core Progress Checkpoint and then uses that accepted Checkpoint as evidence for one bounded graph effect. Progress keeps the current node, completion may advance one declared success route, and failure without a declared failure route pauses in place.
_Avoid_: Work item transition, hidden callback, automatic retry

**Graph rewrite proposal**:
A recorded suggestion to produce a new Graph revision by adding, removing, promoting, collapsing, or reconnecting Control nodes and routes. It never mutates an in-flight Graph revision and material authority changes require explicit human approval.
_Avoid_: self-modifying graph, route decision

**Graph anchor**:
A human-owned goal, frozen rule, or independently evidenced real-world measure that the Control graph cannot rewrite through its own optimization routes. Core capability policy remains an external authority boundary rather than a Graph anchor stored by Board.
_Avoid_: ordinary metric, model preference

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
