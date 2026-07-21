import { board } from "./api.js";
import { createConnectionGesture } from "./connection_gesture.js";
import { canvasLayoutId, moveNode, positionsFromLayout, serializePositions } from "./node_layout.js";

const STATES = ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "cancelled"];
const ACTIVE_STATES = STATES.slice(0, 5);
const NAV = [
  ["overview", "Overview"],
  ["work", "Work"],
  ["projects", "Projects"],
  ["automation", "Automation"],
  ["activity", "Activity"],
];

const ui = {
  page: "overview",
  workspace: null,
  snapshot: null,
  preview: null,
  busy: false,
  graphProjectId: null,
  graphDraft: null,
  graphPositions: new Map(),
  graphConnectSource: null,
  blueprintDraft: null,
  blueprintPositions: new Map(),
  blueprintConnectSource: null,
  blueprintLayoutSaveTimer: null,
};

const root = document.querySelector("#workspace");
const nav = document.querySelector("#primary-nav");
const app = document.querySelector("#app");
const toast = document.querySelector("#toast");

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function label(value) {
  return String(value ?? "")
    .replaceAll("_", " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function clone(value) {
  return structuredClone(value);
}

function announce(message, tone = "normal") {
  toast.textContent = message;
  toast.dataset.tone = tone;
  toast.classList.add("visible");
  window.clearTimeout(announce.timer);
  announce.timer = window.setTimeout(() => toast.classList.remove("visible"), 4200);
}

async function run(action, successMessage) {
  if (ui.busy) return null;
  ui.busy = true;
  app.setAttribute("aria-busy", "true");
  render();
  try {
    const result = await action();
    if (successMessage || result?.message) announce(successMessage || result.message);
    return result;
  } catch (error) {
    announce(String(error), "error");
    return null;
  } finally {
    ui.busy = false;
    app.setAttribute("aria-busy", "false");
    render();
  }
}

async function reload() {
  const snapshot = await board.load();
  ui.snapshot = snapshot;
  document.querySelector("#storage-label").textContent = snapshot.storageLabel;
  document.querySelector("#connection-status").textContent = snapshot.warning
    ? "Local Board · attention"
    : "Local Board · ready";
  document.querySelector("#connection-status").dataset.tone = snapshot.warning ? "warning" : "ready";
  render();
}

function render() {
  root.classList.toggle("fixed-workspace", Boolean(ui.workspace) || ui.page === "work");
  nav.innerHTML = NAV.map(([id, text]) => `
    <button type="button" data-nav="${id}" class="${ui.page === id && !ui.workspace ? "active" : ""}"
      aria-current="${ui.page === id && !ui.workspace ? "page" : "false"}">${text}</button>
  `).join("");

  if (!ui.snapshot) {
    root.innerHTML = `<section class="loading-state"><strong>Opening Gareji Board</strong><span>Reading local coordination state…</span></section>`;
    return;
  }

  if (ui.workspace === "graph") root.innerHTML = renderGraphWorkspace();
  else if (ui.workspace === "blueprint") root.innerHTML = renderBlueprintWorkspace();
  else if (ui.workspace === "portfolio") root.innerHTML = renderPortfolioWorkspace();
  else if (ui.page === "overview") root.innerHTML = renderOverview();
  else if (ui.page === "work") root.innerHTML = renderWork();
  else if (ui.page === "projects") root.innerHTML = renderProjects();
  else if (ui.page === "automation") root.innerHTML = renderAutomation();
  else root.innerHTML = renderActivity();

  if (ui.workspace === "graph") {
    installGraphDragging();
    installGraphConnections();
  }
  if (ui.workspace === "blueprint") {
    installBlueprintDragging();
    installBlueprintConnections();
  }
}

function pageHeader(kicker, title, description, action = "") {
  return `<header class="page-header">
    <div><span class="kicker">${escapeHtml(kicker)}</span><h1>${escapeHtml(title)}</h1><p>${escapeHtml(description)}</p></div>
    ${action}
  </header>`;
}

function renderOverview() {
  const { metrics, projects, workItems, warning } = ui.snapshot;
  const moving = workItems
    .filter((item) => !["done", "cancelled"].includes(item.state))
    .sort((a, b) => a.priority - b.priority)
    .slice(0, 6);
  return `
    ${pageHeader("Operations", "Portfolio overview", "Current work, capacity, and the next explicit control action.",
      `<button class="primary-button" type="button" data-action="preview-autopilot" ${ui.busy ? "disabled" : ""}>Preview Safe Autopilot</button>`)}
    ${warning ? `<div class="notice warning" role="status">${escapeHtml(warning)}</div>` : ""}
    <section class="metrics" aria-label="Portfolio metrics">
      ${metric(metrics.projects, "Projects")}
      ${metric(metrics.activeRuns, "Active runs")}
      ${metric(metrics.blockedItems, "Need attention")}
      ${metric(metrics.deliveryIssues, "Delivery issues")}
    </section>
    ${ui.preview ? renderAutopilotPreview() : ""}
    <section class="overview-grid">
      <article class="desk-panel">
        <header><div><h2>Work in motion</h2><p>Highest-priority non-terminal items</p></div><span>${moving.length} shown</span></header>
        <div class="row-list">${moving.map((item) => `
          <button type="button" class="data-row" data-nav="work">
            <span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.id)} · ${escapeHtml(item.projectId)}</small></span>
            <span class="state-label">${label(item.state)}</span>
          </button>`).join("") || empty("No active Work items")}</div>
      </article>
      <article class="desk-panel">
        <header><div><h2>Project capacity</h2><p>Health and current execution load</p></div><span>${projects.length} managed</span></header>
        <div class="row-list">${projects.map((project) => `
          <button type="button" class="data-row" data-nav="projects">
            <span><strong>${escapeHtml(project.name)}</strong><small>${escapeHtml(project.id)}</small></span>
            <span class="project-state"><i class="health-dot ${project.health}"></i>${label(project.health)} · ${project.activeRuns}/${project.executionCap}</span>
          </button>`).join("")}</div>
      </article>
    </section>`;
}

function metric(value, text) {
  return `<article><strong>${escapeHtml(value)}</strong><span>${text}</span></article>`;
}

function renderAutopilotPreview() {
  const preview = ui.preview;
  const candidate = preview.candidate;
  return `<section class="autopilot-preview" aria-label="Safe Autopilot preview">
    <header><div><span class="kicker">Read-only evaluation</span><h2>Safe Autopilot</h2></div><button class="quiet-button" data-action="close-preview" type="button">Close</button></header>
    <p>${escapeHtml(preview.summary)}</p>
    ${candidate ? `<div class="candidate-row">
      <div><strong>${escapeHtml(candidate.title)}</strong><span>${escapeHtml(candidate.workItemId)} · ${escapeHtml(candidate.projectName)} · ${escapeHtml(candidate.agentRole)}</span></div>
      <div><small>${escapeHtml(candidate.capacity)}</small><button class="primary-button" data-action="start-candidate" data-work-item="${escapeHtml(candidate.workItemId)}" type="button" ${ui.busy ? "disabled" : ""}>Start current Agent Loop</button></div>
    </div>` : ""}
    ${preview.skipped.length ? `<details><summary>${preview.skipped.length} skipped candidate(s)</summary><ul>${preview.skipped.map((item) => `<li><code>${escapeHtml(item.workItemId)}</code> ${escapeHtml(item.reason)}</li>`).join("")}</ul></details>` : ""}
  </section>`;
}

function renderWork() {
  const { projects, workItems } = ui.snapshot;
  return `
    <section class="fixed-page work-page">
    ${pageHeader("Board-owned state", "Work", "Move Work items explicitly; execution outcomes remain separate from lifecycle state.",
      `<div class="header-actions"><button class="quiet-button" type="button" data-action="toggle-agent-form">Add Agent profile</button><button class="secondary-button" type="button" data-action="toggle-work-form">Add Work item</button></div>`)}
    <form id="agent-form" class="inline-form agent-form collapsed" data-form="agent">
      <label>Stable ID<input name="profileId" required placeholder="release-reviewer" /></label>
      <label>Role<input name="role" required placeholder="Release reviewer" /></label>
      <label>Capabilities<input name="capabilities" required placeholder="review, verification" /></label>
      <button class="primary-button" type="submit">Save Agent profile</button>
    </form>
    <form id="work-form" class="inline-form collapsed" data-form="work">
      <label>Project<select name="projectId" required>${projects.map((p) => `<option value="${escapeHtml(p.id)}">${escapeHtml(p.name)}</option>`).join("")}</select></label>
      <label>Stable ID<input name="workItemId" required placeholder="BOARD-12" /></label>
      <label>Title<input name="title" required maxlength="160" /></label>
      <label>Priority<input name="priority" type="number" min="1" value="100" required /></label>
      <button class="primary-button" type="submit">Add to Todo</button>
    </form>
    <section class="kanban" aria-label="Work item board">
      ${ACTIVE_STATES.map((state) => renderLane(state, workItems.filter((item) => item.state === state))).join("")}
    </section>
    </section>`;
}

function renderLane(state, items) {
  return `<section class="kanban-lane" data-lane="${state}">
    <header><span>${label(state)}</span><strong>${items.length}</strong></header>
    <div class="lane-scroll">${items.sort((a, b) => a.priority - b.priority).map((item) => `
      <article class="work-card" draggable="false">
        <span class="work-id">${escapeHtml(item.id)}</span>
        <h2>${escapeHtml(item.title)}</h2>
        <p>${escapeHtml(item.projectId)} · priority ${item.priority}</p>
        <div class="tag-row">${item.agentProfileId ? `<span>${escapeHtml(item.agentProfileId)}</span>` : `<span class="warning-tag">Unassigned</span>`}${item.approvalRequirement !== "none" ? `<span class="warning-tag">Approval</span>` : ""}</div>
        <label class="state-control">Agent<select data-agent-plan data-project="${escapeHtml(item.projectId)}" data-work-item="${escapeHtml(item.id)}" data-expected-agent="${escapeHtml(item.agentProfileId || "")}" data-required="${escapeHtml(item.requiredCapabilities.join(","))}">
          <option value="">Unassigned</option>${ui.snapshot.agentProfiles.map((profile) => `<option value="${escapeHtml(profile.id)}" ${profile.id === item.agentProfileId ? "selected" : ""}>${escapeHtml(profile.role)}</option>`).join("")}
        </select></label>
        <label class="state-control">State<select data-work-state data-project="${escapeHtml(item.projectId)}" data-work-item="${escapeHtml(item.id)}" data-expected="${state}">
          ${STATES.map((option) => `<option value="${option}" ${option === state ? "selected" : ""}>${label(option)}</option>`).join("")}
        </select></label>
      </article>`).join("") || empty("No items")}</div>
  </section>`;
}

function renderProjects() {
  const { projects, executionWorkspaces } = ui.snapshot;
  return `
    ${pageHeader("Portfolio", "Projects", "Each project keeps its own capacity, execution workspace, and graph binding.",
      `<button class="secondary-button" type="button" data-action="toggle-project-form">Add existing project</button>`)}
    <form id="project-form" class="inline-form project-form collapsed" data-form="project">
      <label>Stable ID<input name="projectId" required placeholder="my-product" /></label>
      <label>Name<input name="name" required maxlength="120" /></label>
      <label>Capacity<input name="executionCap" type="number" min="1" value="1" required /></label>
      <label>Local project directory<input name="workspaceLocation" required placeholder="C:\\work\\my-product" /></label>
      <button class="primary-button" type="submit">Connect project</button>
    </form>
    <section class="project-grid">${projects.map((project) => {
      const workspace = executionWorkspaces.find((item) => item.projectId === project.id);
      return `<article class="project-card">
        <header><span class="project-id">${escapeHtml(project.id)}</span><span class="health-label ${project.health}">${label(project.health)}</span></header>
        <h2>${escapeHtml(project.name)}</h2>
        <div class="project-stats"><span><strong>${project.activeRuns}/${project.executionCap}</strong> running</span><span><strong>${project.todoItems}</strong> todo</span><span><strong>${project.blockedItems}</strong> blocked</span></div>
        <div class="workspace-connection"><span>Execution workspace</span><strong title="${escapeHtml(workspace?.location || "")}">${escapeHtml(workspace?.displayName || "Not connected")}</strong></div>
        <button class="secondary-button" type="button" data-open="graph" data-project="${escapeHtml(project.id)}">Open Control Graph</button>
      </article>`;
    }).join("")}</section>`;
}

function renderAutomation() {
  const { controlGraphs, orchestrationBlueprints, portfolioOrchestrations } = ui.snapshot;
  return `
    ${pageHeader("Automation", "Design how work moves", "Automation remains inspectable, versioned, and bounded by human approvals.")}
    <section class="workspace-launcher">
      ${workspaceCard("blueprint", "Blueprint Studio", "Author reusable project-independent approaches and typed connections.", `${orchestrationBlueprints.length} revision(s)`)}
      ${workspaceCard("portfolio", "Portfolio orchestration", "Inspect deterministic coordination across managed projects.", `${portfolioOrchestrations.length} revision(s)`)}
      ${workspaceCard("graph", "Control Graph manager", "Connect Agent loops, gates, audits, approvals, and terminals.", `${controlGraphs.length} revision(s)`)}
    </section>`;
}

function workspaceCard(id, title, description, meta) {
  return `<article class="workspace-card"><span class="kicker">Workspace</span><h2>${title}</h2><p>${description}</p><footer><span>${meta}</span><button class="secondary-button" data-open="${id}" type="button">Open workspace</button></footer></article>`;
}

function ensureGraphDraft() {
  const project = ui.snapshot.projects.find((item) => item.id === ui.graphProjectId) || ui.snapshot.projects[0];
  if (!project) return null;
  ui.graphProjectId = project.id;
  const binding = ui.snapshot.projectGraphBindings.find((item) => item.project_id === project.id);
  const graph = ui.snapshot.controlGraphs.find((item) => binding && item.graph_id === binding.graph_id && item.revision_id === binding.revision_id)
    || ui.snapshot.controlGraphs[0];
  if (!graph) return null;
  if (!ui.graphDraft || ui.graphDraft.graph_id !== graph.graph_id || ui.graphDraft.revision_id !== graph.revision_id) {
    ui.graphDraft = clone(graph);
    ui.graphPositions = positionsForGraph(graph);
    ui.graphConnectSource = null;
  }
  return { project, binding, graph: ui.graphDraft };
}

function positionsForGraph(graph) {
  const saved = ui.snapshot.graphCanvasLayouts.find((item) => item.graph_id === graph.graph_id);
  const map = new Map((saved?.positions || []).map((item) => [item.node_id, { x: item.x, y: item.y }]));
  graph.nodes.forEach((node, index) => {
    if (!map.has(node.id)) map.set(node.id, { x: 46 + (index % 4) * 230, y: 54 + Math.floor(index / 4) * 170 });
  });
  return map;
}

function renderGraphWorkspace() {
  const selected = ensureGraphDraft();
  if (!selected) return workspaceEmpty("Control Graph manager", "Create a project and a graph revision first.");
  const { project, binding, graph } = selected;
  return `<section class="workspace-page">
    ${workspaceHeader("Automation", "Control Graph manager", "Connect bounded stages and publish immutable revisions.")}
    <div class="graph-manager">
      <aside class="master-rail" aria-label="Projects">${ui.snapshot.projects.map((item) => `
        <button type="button" data-select-graph-project="${escapeHtml(item.id)}" class="${item.id === project.id ? "active" : ""}"><strong>${escapeHtml(item.name)}</strong><span>${escapeHtml(item.id)}</span></button>`).join("")}</aside>
      <section class="graph-detail">
        <header class="workspace-toolbar">
          <div><span class="kicker">${escapeHtml(project.name)}</span><h2>${escapeHtml(graph.graph_id)} · ${escapeHtml(graph.revision_id)}</h2></div>
          <label>Graph revision<select data-select-graph>${ui.snapshot.controlGraphs.map((item, index) => `<option value="${index}" ${item.graph_id === graph.graph_id && item.revision_id === graph.revision_id ? "selected" : ""}>${escapeHtml(item.graph_id)} · ${escapeHtml(item.revision_id)}</option>`).join("")}</select></label>
        </header>
        <div class="graph-actions">
          <span>Add stage</span>
          ${["agent_loop", "gate", "audit", "approval", "terminal"].map((kind) => `<button class="quiet-button" type="button" data-add-graph-node="${kind}">${label(kind)}</button>`).join("")}
          <label>Connect signal<select id="graph-signal">${["succeeded", "failed", "passed", "approved", "rejected"].map((signal) => `<option value="${signal}">${label(signal)}</option>`).join("")}</select></label>
          <span class="connect-status">${ui.graphConnectSource ? `Connecting from ${escapeHtml(ui.graphConnectSource)} — select a destination` : "Select a node, then another node, to connect"}</span>
        </div>
        ${renderControlCanvas(graph)}
        <div class="revision-bar">
          <label>New immutable revision<input id="graph-revision-id" value="${escapeHtml(nextRevisionId(graph.revision_id))}" /></label>
          <button class="secondary-button" data-action="bind-graph" type="button">Use this graph for ${escapeHtml(project.name)}</button>
          <button class="primary-button" data-action="publish-graph" type="button">Publish revision</button>
        </div>
        <details class="route-list"><summary>${graph.routes.length} declared route(s)</summary>${graph.routes.map((route) => `<div><code>${escapeHtml(route.source_node_id)}</code><span>${label(route.signal)}</span><code>${escapeHtml(route.destination_node_id)}</code><button type="button" data-remove-route="${escapeHtml(route.id)}">Remove</button></div>`).join("")}</details>
        ${binding ? `<p class="supporting-text">Project binding: ${escapeHtml(binding.graph_id)} · ${escapeHtml(binding.revision_id)} · entry ${escapeHtml(binding.entry_id)}</p>` : ""}
      </section>
    </div>
  </section>`;
}

function renderControlCanvas(graph) {
  const lines = graph.routes.map((route) => {
    const source = ui.graphPositions.get(route.source_node_id) || { x: 0, y: 0 };
    const target = ui.graphPositions.get(route.destination_node_id) || { x: 0, y: 0 };
    return `<path d="M ${source.x + 176} ${source.y + 46} C ${source.x + 215} ${source.y + 46}, ${target.x - 35} ${target.y + 46}, ${target.x} ${target.y + 46}" /><text x="${(source.x + target.x + 176) / 2}" y="${(source.y + target.y + 76) / 2}">${escapeHtml(route.signal)}</text>`;
  }).join("");
  return `<div class="graph-scroll"><div class="graph-canvas" data-graph-canvas>
    <svg viewBox="0 0 1200 650" aria-hidden="true" data-graph-links>${lines}</svg>
    ${graph.nodes.map((node) => {
      const point = ui.graphPositions.get(node.id) || { x: 0, y: 0 };
      const active = ui.graphConnectSource === node.id;
      return `<div class="graph-node ${active ? "connecting" : ""}" data-graph-node="${escapeHtml(node.id)}" style="left:${point.x}px;top:${point.y}px">
        <button type="button" class="node-body graph-node-body" data-graph-select="${escapeHtml(node.id)}">
          <span>${graph.entries.some((entry) => entry.node_id === node.id) ? "ENTRY" : graphKind(node.kind)}</span><strong>${escapeHtml(node.id)}</strong><small>${escapeHtml(graphKindDetail(node.kind))}</small>
        </button>
        <button type="button" class="node-port node-port-input" data-graph-input="${escapeHtml(node.id)}" aria-label="Connect into ${escapeHtml(node.id)}"></button>
        <button type="button" class="node-port node-port-output" data-graph-output="${escapeHtml(node.id)}" aria-label="Connect from ${escapeHtml(node.id)}"></button>
      </div>`;
    }).join("")}
  </div></div>`;
}

function graphKind(kind) {
  return typeof kind === "string" ? label(kind) : label(kind?.kind || "stage");
}

function graphKindDetail(kind) {
  return kind?.agent_profile_id ? `Agent · ${kind.agent_profile_id}` : graphKind(kind);
}

function ensureBlueprintDraft() {
  if (!ui.blueprintDraft) {
    ui.blueprintDraft = clone(ui.snapshot.orchestrationBlueprints[0] || null);
    ui.blueprintPositions = ui.blueprintDraft
      ? positionsForBlueprint(ui.blueprintDraft)
      : new Map();
    ui.blueprintConnectSource = null;
  }
  return ui.blueprintDraft;
}

function blueprintCanvasLayoutId(blueprint) {
  return canvasLayoutId("blueprint", blueprint.blueprint_id);
}

function positionsForBlueprint(blueprint) {
  const layoutId = blueprintCanvasLayoutId(blueprint);
  const saved = ui.snapshot.graphCanvasLayouts.find((item) => item.graph_id === layoutId);
  return positionsFromLayout(blueprint.nodes, saved);
}

function renderBlueprintWorkspace() {
  const blueprint = ensureBlueprintDraft();
  if (!blueprint) return workspaceEmpty("Blueprint Studio", "No Blueprint revision is available.");
  return `<section class="workspace-page">
    ${workspaceHeader("Automation", "Blueprint Studio", "Author reusable approaches without fixing a project or Runner.")}
    <div class="blueprint-toolbar"><label>Revision<select data-select-blueprint>${ui.snapshot.orchestrationBlueprints.map((item, index) => `<option value="${index}" ${item.blueprint_id === blueprint.blueprint_id && item.revision_id === blueprint.revision_id ? "selected" : ""}>${escapeHtml(item.name)} · ${escapeHtml(item.revision_id)}</option>`).join("")}</select></label><span>${ui.blueprintConnectSource ? `Connecting from ${escapeHtml(ui.blueprintConnectSource)}` : "Drag cards to arrange · drag ports to connect · arrow keys nudge"}</span></div>
    <div class="blueprint-grid">
      <aside class="blueprint-notes"><header><span>Approach notes</span><strong>${ui.snapshot.approachNotes.length}</strong></header>${ui.snapshot.approachNotes.map((note) => `<article><span class="risk ${note.risk}">${escapeHtml(note.risk)}</span><h2>${escapeHtml(note.title)}</h2><p>${escapeHtml(note.filename)}</p><div class="tag-row">${note.inputs.concat(note.outputs).map((socket) => `<span>${label(socket)}</span>`).join("")}</div><button class="secondary-button" type="button" data-add-approach="${escapeHtml(note.approachId)}">Add to graph</button></article>`).join("") || empty("No Approach Notes")}</aside>
      <section class="blueprint-canvas-panel"><header><div><span class="kicker">${escapeHtml(blueprint.blueprint_id)} · ${escapeHtml(blueprint.revision_id)}</span><h2>${escapeHtml(blueprint.name)}</h2></div><div>${["gate", "audit", "approval", "terminal"].map((kind) => `<button type="button" class="quiet-button" data-add-blueprint-node="${kind}">${label(kind)}</button>`).join("")}</div></header>${renderBlueprintCanvas(blueprint)}</section>
      <aside class="blueprint-inspector"><span class="kicker">Inspector</span><h2>Portable contract</h2><dl><div><dt>Scope</dt><dd>${label(blueprint.scope)}</dd></div><div><dt>Entry</dt><dd>${escapeHtml(blueprint.entry_node_id)}</dd></div><div><dt>Nodes</dt><dd>${blueprint.nodes.length}</dd></div><div><dt>Connections</dt><dd>${blueprint.links.length}</dd></div></dl><label>New revision<input id="blueprint-revision-id" value="${escapeHtml(nextRevisionId(blueprint.revision_id))}" /></label><button class="primary-button" type="button" data-action="publish-blueprint">Publish immutable revision</button><div class="link-list">${blueprint.links.map((link) => `<div><span>${escapeHtml(link.source_node_id)} → ${escapeHtml(link.destination_node_id)}</span><button type="button" data-remove-blueprint-link="${escapeHtml(link.id)}">Remove</button></div>`).join("")}</div></aside>
    </div>
  </section>`;
}

function renderBlueprintCanvas(blueprint) {
  return `<div class="blueprint-canvas-viewport"><div class="blueprint-canvas" data-blueprint-canvas>
    <svg aria-hidden="true" data-blueprint-links></svg>
    ${blueprint.nodes.map((node) => {
      const point = ui.blueprintPositions.get(node.id) || { x: 46, y: 54 };
      return `<div data-blueprint-node-container="${escapeHtml(node.id)}" class="blueprint-node ${ui.blueprintConnectSource === node.id ? "connecting" : ""}" style="left:${point.x}px;top:${point.y}px">
      <button type="button" class="node-body blueprint-node-body" data-blueprint-node="${escapeHtml(node.id)}" aria-label="${escapeHtml(node.id)}. Drag to move, or use arrow keys. Press to select for connection." title="Drag to move · arrow keys to nudge"><span>${blueprint.entry_node_id === node.id ? "ENTRY" : graphKind(node.kind)}</span><strong>${escapeHtml(node.id)}</strong><small>${node.inputs?.length || 0} in · ${node.outputs?.length || 0} out</small></button>
      <button type="button" class="node-port node-port-input" data-blueprint-input="${escapeHtml(node.id)}" aria-label="Connect into ${escapeHtml(node.id)}"></button>
      <button type="button" class="node-port node-port-output" data-blueprint-output="${escapeHtml(node.id)}" aria-label="Connect from ${escapeHtml(node.id)}"></button>
    </div>`;
    }).join("")}
  </div></div>`;
}

function renderPortfolioWorkspace() {
  const revisions = ui.snapshot.portfolioOrchestrations;
  const revision = revisions[0];
  if (!revision) return workspaceEmpty("Portfolio orchestration", "No Portfolio orchestration revision is available.");
  return `<section class="workspace-page">${workspaceHeader("Automation", "Portfolio orchestration", "One bounded node advances per tick; approval remains human-owned.")}<div class="portfolio-workspace"><header><span class="kicker">${escapeHtml(revision.orchestration_id)} · ${escapeHtml(revision.revision_id)}</span><h2>${escapeHtml(revision.name)}</h2></header><div class="portfolio-flow">${(revision.nodes || []).map((node, index) => `<article><span>${index + 1}</span><strong>${escapeHtml(node.id)}</strong><small>${graphKind(node.kind)}</small></article>`).join("")}</div><p class="supporting-text">This surface is intentionally read-only. The local scheduler advances the immutable revision without starting a Project Runner or bypassing Approval nodes.</p></div></section>`;
}

function renderActivity() {
  const { activity, metrics } = ui.snapshot;
  return `${pageHeader("Evidence", "Activity", "Accepted Progress Checkpoints remain immutable and recommend rather than own Work item state.")}<section class="activity-list"><header><span>${metrics.inboxItems} inbox</span><span>${metrics.deliveryIssues} delivery issue(s)</span></header>${activity.map((item) => {
    const targets = ui.snapshot.workItems.filter((work) => work.projectId === item.projectId);
    const recommendation = item.recommendedState && item.workItemId && !item.reconciliation
      ? `<div class="activity-actions"><span>Recommendation: ${label(item.recommendedState)}</span><button class="quiet-button" type="button" data-reconcile="dismissed" data-checkpoint="${escapeHtml(item.checkpointId)}" data-project="${escapeHtml(item.projectId)}" data-work-item="${escapeHtml(item.workItemId)}" data-state="${escapeHtml(item.recommendedState)}">Dismiss</button><button class="secondary-button" type="button" data-reconcile="accepted" data-checkpoint="${escapeHtml(item.checkpointId)}" data-project="${escapeHtml(item.projectId)}" data-work-item="${escapeHtml(item.workItemId)}" data-state="${escapeHtml(item.recommendedState)}">Accept</button></div>`
      : item.reconciliation ? `<small>Recommendation ${escapeHtml(item.reconciliation)}</small>` : "";
    const inbox = !item.workItemId && targets.length
      ? `<div class="activity-actions"><label>Attach to<select data-activity-target="${escapeHtml(item.checkpointId)}">${targets.map((work) => `<option value="${escapeHtml(work.id)}">${escapeHtml(work.id)} · ${escapeHtml(work.title)}</option>`).join("")}</select></label><button class="secondary-button" type="button" data-attach-checkpoint="${escapeHtml(item.checkpointId)}" data-project="${escapeHtml(item.projectId)}">Attach</button></div>` : "";
    return `<article><div><span class="outcome ${escapeHtml(item.outcome.toLowerCase().replaceAll(" ", "-"))}">${escapeHtml(item.outcome)}</span><time>${escapeHtml(item.recordedAt)}</time></div><h2>${escapeHtml(item.summary)}</h2><p>${escapeHtml(item.projectId)}${item.workItemId ? ` · ${escapeHtml(item.workItemId)}` : " · Inbox"} · ${escapeHtml(item.source)}</p>${recommendation}${inbox}</article>`;
  }).join("") || empty("No Progress Checkpoints")}</section>`;
}

function workspaceHeader(parent, title, description) {
  return `<header class="workspace-header"><button type="button" class="back-button" data-action="back-automation">Back to ${escapeHtml(parent)}</button><div><span class="kicker">Focused workspace</span><h1>${escapeHtml(title)}</h1><p>${escapeHtml(description)}</p></div></header>`;
}

function workspaceEmpty(title, message) {
  return `<section class="workspace-page">${workspaceHeader("Automation", title, message)}${empty(message)}</section>`;
}

function empty(message) {
  return `<p class="empty-state">${escapeHtml(message)}</p>`;
}

function nextRevisionId(current) {
  const match = String(current).match(/^(.*?)(\d+)$/);
  return match ? `${match[1]}${Number(match[2]) + 1}` : `${current}-next`;
}

function uniqueNodeId(nodes, prefix) {
  let index = 1;
  while (nodes.some((node) => node.id === `${prefix}-${index}`)) index += 1;
  return `${prefix}-${index}`;
}

function addGraphNode(kind) {
  const graph = ui.graphDraft;
  const id = uniqueNodeId(graph.nodes, kind.replace("_loop", ""));
  const nodeKind = kind === "agent_loop"
    ? { kind, agent_profile_id: ui.snapshot.agentProfiles[0]?.id || "unassigned-agent" }
    : { kind };
  graph.nodes.push({ id, kind: nodeKind });
  ui.graphPositions.set(id, { x: 60 + (graph.nodes.length % 4) * 220, y: 80 + Math.floor(graph.nodes.length / 4) * 160 });
  render();
}

function connectGraphNode(nodeId) {
  if (!ui.graphConnectSource) {
    ui.graphConnectSource = nodeId;
  } else if (ui.graphConnectSource === nodeId) {
    ui.graphConnectSource = null;
  } else {
    const source = ui.graphConnectSource;
    commitGraphConnection(source, nodeId);
    return;
  }
  render();
}

function commitGraphConnection(source, destination) {
  const signal = document.querySelector("#graph-signal")?.value || "succeeded";
  ui.graphDraft.routes = ui.graphDraft.routes.filter((route) => !(route.source_node_id === source && route.signal === signal));
  ui.graphDraft.routes.push({ id: `${source}-${signal}-${destination}`, source_node_id: source, destination_node_id: destination, signal });
  ui.graphConnectSource = null;
  render();
}

function addBlueprintNode(kind, approachId = null) {
  const blueprint = ui.blueprintDraft;
  const id = uniqueNodeId(blueprint.nodes, approachId || kind);
  let nodeKind = { kind };
  let inputs = [];
  let outputs = [];
  if (approachId) {
    const note = ui.snapshot.approachNotes.find((item) => item.approachId === approachId);
    nodeKind = { kind: "approach", approach_id: approachId };
    inputs = note?.inputs || [];
    outputs = note?.outputs || [];
  }
  blueprint.nodes.push({ id, kind: nodeKind, inputs, outputs });
  const index = blueprint.nodes.length - 1;
  ui.blueprintPositions.set(id, {
    x: 46 + (index % 4) * 230,
    y: 54 + Math.floor(index / 4) * 170,
  });
  render();
}

function connectBlueprintNode(nodeId) {
  if (!ui.blueprintConnectSource) ui.blueprintConnectSource = nodeId;
  else if (ui.blueprintConnectSource === nodeId) ui.blueprintConnectSource = null;
  else {
    const source = ui.blueprintConnectSource;
    commitBlueprintConnection(source, nodeId);
    return;
  }
  render();
}

function commitBlueprintConnection(source, destination) {
  blueprintRemoveFlow(ui.blueprintDraft, source, "succeeded");
  ui.blueprintDraft.links.push({ id: `${source}-succeeded-${destination}`, source_node_id: source, destination_node_id: destination, kind: { kind: "flow", signal: "succeeded" } });
  ui.blueprintConnectSource = null;
  render();
}

function blueprintRemoveFlow(blueprint, source, signal) {
  blueprint.links = blueprint.links.filter((link) => !(link.source_node_id === source && link.kind?.kind === "flow" && link.kind.signal === signal));
}

function openWorkspace(name, projectId = null) {
  ui.page = "automation";
  ui.workspace = name;
  if (name === "graph") {
    ui.graphProjectId = projectId || ui.snapshot.projects[0]?.id || null;
    ui.graphDraft = null;
  }
  if (name === "blueprint") {
    ui.blueprintDraft = null;
    ui.blueprintPositions = new Map();
  }
  render();
  root.focus();
}

nav.addEventListener("click", (event) => {
  const target = event.target.closest("button[data-nav]");
  if (!target) return;
  ui.page = target.dataset.nav;
  ui.workspace = null;
  render();
  root.focus();
});

root.addEventListener("click", async (event) => {
  const target = event.target.closest("button");
  if (!target) return;
  if (target.dataset.nav) {
    ui.page = target.dataset.nav;
    ui.workspace = null;
    render();
  } else if (target.dataset.open) {
    openWorkspace(target.dataset.open, target.dataset.project);
  } else if (target.dataset.action === "back-automation") {
    ui.workspace = null;
    ui.page = "automation";
    render();
  } else if (target.dataset.action === "preview-autopilot") {
    ui.preview = await run(() => board.previewAutopilot());
    render();
  } else if (target.dataset.action === "close-preview") {
    ui.preview = null;
    render();
  } else if (target.dataset.action === "start-candidate") {
    const result = await run(() => board.startAgentLoop({ workItemId: target.dataset.workItem }));
    if (result) await reload();
  } else if (target.dataset.action === "toggle-work-form") {
    document.querySelector("#work-form")?.classList.toggle("collapsed");
  } else if (target.dataset.action === "toggle-agent-form") {
    document.querySelector("#agent-form")?.classList.toggle("collapsed");
  } else if (target.dataset.action === "toggle-project-form") {
    document.querySelector("#project-form")?.classList.toggle("collapsed");
  } else if (target.dataset.selectGraphProject) {
    ui.graphProjectId = target.dataset.selectGraphProject;
    ui.graphDraft = null;
    render();
  } else if (target.dataset.addGraphNode) {
    addGraphNode(target.dataset.addGraphNode);
  } else if (target.dataset.graphSelect) {
    connectGraphNode(target.dataset.graphSelect);
  } else if (target.dataset.removeRoute) {
    ui.graphDraft.routes = ui.graphDraft.routes.filter((route) => route.id !== target.dataset.removeRoute);
    render();
  } else if (target.dataset.action === "publish-graph") {
    const revisionId = document.querySelector("#graph-revision-id")?.value.trim();
    if (!revisionId) return announce("Enter a new revision ID.", "error");
    const graph = clone(ui.graphDraft);
    graph.revision_id = revisionId;
    const result = await run(() => board.publishGraph(graph));
    if (result) { await reload(); ui.graphDraft = graph; }
  } else if (target.dataset.action === "bind-graph") {
    const projectId = ui.graphProjectId;
    const expected = ui.snapshot.projectGraphBindings.find((item) => item.project_id === projectId) || null;
    const entry = ui.graphDraft.entries[0];
    if (!entry) return announce("The graph has no declared entry.", "error");
    const result = await run(() => board.bindProjectGraph({ expected, target: { project_id: projectId, graph_id: ui.graphDraft.graph_id, revision_id: ui.graphDraft.revision_id, entry_id: entry.id } }));
    if (result) await reload();
  } else if (target.dataset.addApproach) {
    addBlueprintNode("approach", target.dataset.addApproach);
  } else if (target.dataset.addBlueprintNode) {
    addBlueprintNode(target.dataset.addBlueprintNode);
  } else if (target.dataset.blueprintNode) {
    connectBlueprintNode(target.dataset.blueprintNode);
  } else if (target.dataset.removeBlueprintLink) {
    ui.blueprintDraft.links = ui.blueprintDraft.links.filter((link) => link.id !== target.dataset.removeBlueprintLink);
    render();
  } else if (target.dataset.action === "publish-blueprint") {
    const revisionId = document.querySelector("#blueprint-revision-id")?.value.trim();
    if (!revisionId) return announce("Enter a new revision ID.", "error");
    const blueprint = clone(ui.blueprintDraft);
    blueprint.revision_id = revisionId;
    const result = await run(() => board.publishBlueprint(blueprint));
    if (result) { await reload(); ui.blueprintDraft = blueprint; }
  } else if (target.dataset.reconcile) {
    const request = { checkpointId: target.dataset.checkpoint, projectId: target.dataset.project, workItemId: target.dataset.workItem, recommendedState: target.dataset.state, decision: target.dataset.reconcile };
    const result = await run(() => board.reconcileActivity(request));
    if (result) await reload();
  } else if (target.dataset.attachCheckpoint) {
    const select = document.querySelector(`[data-activity-target="${CSS.escape(target.dataset.attachCheckpoint)}"]`);
    const workItemId = select?.value;
    if (!workItemId) return announce("Choose a Work item target.", "error");
    const result = await run(() => board.attachActivity({ checkpointId: target.dataset.attachCheckpoint, projectId: target.dataset.project, workItemId }));
    if (result) await reload();
  }
});

root.addEventListener("change", async (event) => {
  const select = event.target;
  if (select.matches("[data-work-state]")) {
    const request = { projectId: select.dataset.project, workItemId: select.dataset.workItem, expectedState: select.dataset.expected, targetState: select.value };
    const result = await run(() => board.transitionWorkItem(request));
    if (result) await reload();
  } else if (select.matches("[data-agent-plan]")) {
    const request = { projectId: select.dataset.project, workItemId: select.dataset.workItem, expectedAgentProfileId: select.dataset.expectedAgent || null, targetAgentProfileId: select.value || null, requiredCapabilities: (select.dataset.required || "").split(",").filter(Boolean) };
    const result = await run(() => board.updateAgentPlan(request));
    if (result) await reload();
  } else if (select.matches("[data-select-graph]")) {
    const graph = ui.snapshot.controlGraphs[Number(select.value)];
    ui.graphDraft = clone(graph);
    ui.graphPositions = positionsForGraph(graph);
    ui.graphConnectSource = null;
    render();
  } else if (select.matches("[data-select-blueprint]")) {
    ui.blueprintDraft = clone(ui.snapshot.orchestrationBlueprints[Number(select.value)]);
    ui.blueprintPositions = positionsForBlueprint(ui.blueprintDraft);
    ui.blueprintConnectSource = null;
    render();
  }
});

root.addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.target;
  const values = Object.fromEntries(new FormData(form));
  if (form.dataset.form === "work") {
    const result = await run(() => board.createWorkItem({ ...values, priority: Number(values.priority) }));
    if (result) { form.reset(); await reload(); }
  } else if (form.dataset.form === "project") {
    const result = await run(() => board.createProject({ ...values, executionCap: Number(values.executionCap) }));
    if (result) { form.reset(); await reload(); }
  } else if (form.dataset.form === "agent") {
    const capabilities = String(values.capabilities).split(",").map((value) => value.trim()).filter(Boolean);
    const result = await run(() => board.saveAgentProfile({ profileId: values.profileId, role: values.role, capabilities }));
    if (result) { form.reset(); await reload(); }
  }
});

function installGraphDragging() {
  document.querySelectorAll("[data-graph-node]").forEach((node) => {
    const body = node.querySelector(".graph-node-body");
    if (!body) return;
    body.addEventListener("click", (event) => {
      if (body.dataset.suppressClick !== "true") return;
      delete body.dataset.suppressClick;
      event.preventDefault();
      event.stopPropagation();
    });
    body.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const id = node.dataset.graphNode;
      const start = ui.graphPositions.get(id);
      const origin = { x: event.clientX, y: event.clientY };
      let moved = false;
      body.setPointerCapture(event.pointerId);
      const move = (next) => {
        const dx = next.clientX - origin.x;
        const dy = next.clientY - origin.y;
        if (Math.abs(dx) + Math.abs(dy) < 5) return;
        moved = true;
        const point = { x: Math.max(12, start.x + dx), y: Math.max(12, start.y + dy) };
        node.style.left = `${point.x}px`;
        node.style.top = `${point.y}px`;
        ui.graphPositions.set(id, point);
      };
      const up = async () => {
        body.removeEventListener("pointermove", move);
        body.removeEventListener("pointerup", up);
        if (!moved) return;
        body.dataset.suppressClick = "true";
        const layout = { graph_id: ui.graphDraft.graph_id, positions: [...ui.graphPositions].map(([node_id, point]) => ({ node_id, x: point.x, y: point.y })) };
        await run(() => board.saveGraphLayout(layout));
      };
      body.addEventListener("pointermove", move);
      body.addEventListener("pointerup", up);
    });
  });
}

function installGraphConnections() {
  installConnectionDragging({
    canvas: document.querySelector("[data-graph-canvas]"),
    svg: document.querySelector("[data-graph-links]"),
    outputSelector: "[data-graph-output]",
    inputSelector: "[data-graph-input]",
    sourceDataset: "graphOutput",
    destinationDataset: "graphInput",
    onSourceClick: connectGraphNode,
    onDestinationClick: (destination) => {
      if (ui.graphConnectSource) connectGraphNode(destination);
    },
    onCommit: ({ sourceId, destinationId }) => commitGraphConnection(sourceId, destinationId),
  });
}

function blueprintLayout() {
  if (!ui.blueprintDraft) return null;
  return serializePositions(
    blueprintCanvasLayoutId(ui.blueprintDraft),
    ui.blueprintPositions,
  );
}

function rememberLayout(layout) {
  const layouts = ui.snapshot.graphCanvasLayouts;
  const index = layouts.findIndex((item) => item.graph_id === layout.graph_id);
  if (index >= 0) layouts[index] = clone(layout);
  else layouts.push(clone(layout));
}

async function saveBlueprintLayoutWithFeedback() {
  const layout = blueprintLayout();
  if (!layout) return;
  window.clearTimeout(ui.blueprintLayoutSaveTimer);
  ui.blueprintLayoutSaveTimer = null;
  const result = await run(() => board.saveGraphLayout(layout), "Blueprint layout saved.");
  if (result) rememberLayout(layout);
}

function scheduleBlueprintLayoutSave() {
  const layout = blueprintLayout();
  if (!layout) return;
  window.clearTimeout(ui.blueprintLayoutSaveTimer);
  ui.blueprintLayoutSaveTimer = window.setTimeout(async () => {
    try {
      await board.saveGraphLayout(layout);
      rememberLayout(layout);
    } catch (error) {
      announce(`Blueprint layout was not saved: ${error}`, "error");
    }
  }, 260);
}

function blueprintDragBounds(canvas, node) {
  return {
    canvasWidth: canvas.scrollWidth,
    canvasHeight: canvas.scrollHeight,
    nodeWidth: node.offsetWidth,
    nodeHeight: node.offsetHeight,
  };
}

function placeBlueprintNode(canvas, svg, node, id, point) {
  node.style.left = `${point.x}px`;
  node.style.top = `${point.y}px`;
  ui.blueprintPositions.set(id, point);
  redrawBlueprintLinks(canvas, svg);
}

function installBlueprintDragging() {
  const canvas = document.querySelector("[data-blueprint-canvas]");
  const svg = document.querySelector("[data-blueprint-links]");
  if (!canvas || !svg) return;

  canvas.querySelectorAll("[data-blueprint-node-container]").forEach((node) => {
    const body = node.querySelector(".blueprint-node-body");
    if (!body) return;

    body.addEventListener("click", (event) => {
      if (body.dataset.suppressClick !== "true") return;
      delete body.dataset.suppressClick;
      event.preventDefault();
      event.stopPropagation();
    });

    body.addEventListener("keydown", (event) => {
      const directions = {
        ArrowLeft: { x: -1, y: 0 },
        ArrowRight: { x: 1, y: 0 },
        ArrowUp: { x: 0, y: -1 },
        ArrowDown: { x: 0, y: 1 },
      };
      const direction = directions[event.key];
      if (!direction) return;
      event.preventDefault();
      const id = node.dataset.blueprintNodeContainer;
      const start = ui.blueprintPositions.get(id) || { x: 46, y: 54 };
      const distance = event.shiftKey ? 40 : 10;
      const point = moveNode(
        start,
        { x: 0, y: 0 },
        { x: direction.x * distance, y: direction.y * distance },
        blueprintDragBounds(canvas, node),
      );
      placeBlueprintNode(canvas, svg, node, id, point);
      scheduleBlueprintLayoutSave();
    });

    body.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const id = node.dataset.blueprintNodeContainer;
      const start = ui.blueprintPositions.get(id) || { x: 46, y: 54 };
      const origin = { x: event.clientX, y: event.clientY };
      let moved = false;
      body.setPointerCapture(event.pointerId);

      const move = (next) => {
        if (next.pointerId !== event.pointerId) return;
        const dx = next.clientX - origin.x;
        const dy = next.clientY - origin.y;
        if (!moved && Math.abs(dx) + Math.abs(dy) < 5) return;
        moved = true;
        node.classList.add("dragging");
        const point = moveNode(
          start,
          origin,
          { x: next.clientX, y: next.clientY },
          blueprintDragBounds(canvas, node),
        );
        placeBlueprintNode(canvas, svg, node, id, point);
      };

      const cleanup = () => {
        body.removeEventListener("pointermove", move);
        body.removeEventListener("pointerup", finish);
        body.removeEventListener("pointercancel", cancel);
        node.classList.remove("dragging");
        if (body.hasPointerCapture(event.pointerId)) body.releasePointerCapture(event.pointerId);
      };

      const finish = async (next) => {
        if (next.pointerId !== event.pointerId) return;
        cleanup();
        if (!moved) return;
        body.dataset.suppressClick = "true";
        await saveBlueprintLayoutWithFeedback();
      };

      const cancel = (next) => {
        if (next.pointerId !== event.pointerId) return;
        cleanup();
        if (moved) placeBlueprintNode(canvas, svg, node, id, start);
      };

      body.addEventListener("pointermove", move);
      body.addEventListener("pointerup", finish);
      body.addEventListener("pointercancel", cancel);
    });
  });
}

function installBlueprintConnections() {
  const canvas = document.querySelector("[data-blueprint-canvas]");
  const svg = document.querySelector("[data-blueprint-links]");
  if (!canvas || !svg) return;
  redrawBlueprintLinks(canvas, svg);
  installConnectionDragging({
    canvas,
    svg,
    outputSelector: "[data-blueprint-output]",
    inputSelector: "[data-blueprint-input]",
    sourceDataset: "blueprintOutput",
    destinationDataset: "blueprintInput",
    onSourceClick: connectBlueprintNode,
    onDestinationClick: (destination) => {
      if (ui.blueprintConnectSource) connectBlueprintNode(destination);
    },
    onCommit: ({ sourceId, destinationId }) => commitBlueprintConnection(sourceId, destinationId),
  });
}

function installConnectionDragging({ canvas, svg, outputSelector, inputSelector, sourceDataset, destinationDataset, onSourceClick, onDestinationClick, onCommit }) {
  if (!canvas || !svg) return;
  const gesture = createConnectionGesture({
    onPreview: (preview) => drawConnectionPreview(canvas, svg, outputSelector, sourceDataset, preview),
    onCommit,
  });

  canvas.querySelectorAll(outputSelector).forEach((port) => {
    port.addEventListener("click", (event) => {
      event.stopPropagation();
      if (port.dataset.suppressClick === "true") {
        delete port.dataset.suppressClick;
        event.preventDefault();
        return;
      }
      onSourceClick(port.dataset[sourceDataset]);
    });
    port.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      const sourceId = port.dataset[sourceDataset];
      const origin = { x: event.clientX, y: event.clientY };
      let moved = false;
      gesture.start({ sourceId, pointerId: event.pointerId, point: portPoint(canvas, port) });

      const move = (next) => {
        if (next.pointerId !== event.pointerId) return;
        if (Math.abs(next.clientX - origin.x) + Math.abs(next.clientY - origin.y) >= 5) moved = true;
        gesture.move({ pointerId: next.pointerId, point: canvasPoint(canvas, next.clientX, next.clientY) });
      };
      const cleanup = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", finish);
        window.removeEventListener("pointercancel", cancel);
      };
      const finish = (next) => {
        if (next.pointerId !== event.pointerId) return;
        const destination = document.elementFromPoint(next.clientX, next.clientY)?.closest(inputSelector)?.dataset[destinationDataset] || null;
        cleanup();
        const committed = gesture.finish({ pointerId: next.pointerId, destinationId: destination });
        if (moved || committed) port.dataset.suppressClick = "true";
      };
      const cancel = (next) => {
        if (next.pointerId !== event.pointerId) return;
        cleanup();
        gesture.cancel(next.pointerId);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", finish);
      window.addEventListener("pointercancel", cancel);
    });
  });

  canvas.querySelectorAll(inputSelector).forEach((port) => {
    port.addEventListener("click", (event) => {
      event.stopPropagation();
      onDestinationClick(port.dataset[destinationDataset]);
    });
  });
}

function redrawBlueprintLinks(canvas, svg) {
  const width = Math.max(canvas.clientWidth, canvas.scrollWidth);
  const height = Math.max(canvas.clientHeight, canvas.scrollHeight);
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.replaceChildren();
  ui.blueprintDraft.links.forEach((link) => {
    const source = canvas.querySelector(`[data-blueprint-output="${CSS.escape(link.source_node_id)}"]`);
    const destination = canvas.querySelector(`[data-blueprint-input="${CSS.escape(link.destination_node_id)}"]`);
    if (!source || !destination) return;
    svg.append(createConnectionPath(portPoint(canvas, source), portPoint(canvas, destination), "connection-line"));
  });
}

function drawConnectionPreview(canvas, svg, outputSelector, sourceDataset, preview) {
  svg.querySelector(".connection-preview")?.remove();
  if (!preview) return;
  const source = [...canvas.querySelectorAll(outputSelector)].find((port) => port.dataset[sourceDataset] === preview.sourceId);
  if (!source) return;
  svg.append(createConnectionPath(portPoint(canvas, source), preview.point, "connection-preview"));
}

function createConnectionPath(source, destination, className) {
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  const reach = Math.max(55, Math.abs(destination.x - source.x) * 0.42);
  path.setAttribute("class", className);
  path.setAttribute("d", `M ${source.x} ${source.y} C ${source.x + reach} ${source.y}, ${destination.x - reach} ${destination.y}, ${destination.x} ${destination.y}`);
  return path;
}

function portPoint(canvas, port) {
  const canvasRect = canvas.getBoundingClientRect();
  const portRect = port.getBoundingClientRect();
  return {
    x: portRect.left + portRect.width / 2 - canvasRect.left + canvas.scrollLeft,
    y: portRect.top + portRect.height / 2 - canvasRect.top + canvas.scrollTop,
  };
}

function canvasPoint(canvas, clientX, clientY) {
  const rect = canvas.getBoundingClientRect();
  return { x: clientX - rect.left + canvas.scrollLeft, y: clientY - rect.top + canvas.scrollTop };
}

reload().catch((error) => {
  app.setAttribute("aria-busy", "false");
  root.innerHTML = `<section class="fatal-state"><h1>Gareji Board could not open</h1><p>${escapeHtml(error)}</p><button type="button" onclick="location.reload()">Try again</button></section>`;
});
