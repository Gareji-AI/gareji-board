# Gareji Board CLI

The `gareji-board` binary uses the same local SQLite database and Board Modules as the
desktop app. Override the database with `--database PATH` or `GAREJI_BOARD_DB`.

## Headless Portfolio scheduling

Run a persistent scheduler without opening the desktop UI:

```powershell
cargo run -p gareji-board-bridge --bin gareji-board -- portfolio-daemon --poll-interval-seconds 30
```

The polling interval is not a workflow cadence. Each immutable Portfolio revision
still decides when it is due. One bounded diagnostic pass remains available as
`portfolio-tick-due`.

Install or remove the current-user Windows background task:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/install-portfolio-scheduler.ps1
powershell -ExecutionPolicy Bypass -File scripts/uninstall-portfolio-scheduler.ps1
```

## Inspect definitions

```powershell
gareji-board control-graph list
gareji-board control-graph show --graph-id delivery --revision-id v1
gareji-board blueprint list
gareji-board blueprint show --blueprint-id evidence-first --revision-id v1
```

All output is JSON. `show` always requires an exact immutable revision identity.

## Publish a Control Graph edit plan

```json
{
  "graph_id": "delivery",
  "source_revision_id": "v1",
  "new_revision_id": "v2",
  "operations": [
    { "operation": "remove_route", "route_id": "implement-finish" },
    {
      "operation": "add_node",
      "node": { "id": "audit", "kind": { "kind": "audit" } }
    },
    {
      "operation": "connect",
      "route": {
        "id": "implement-audit",
        "source_node_id": "implement",
        "destination_node_id": "audit",
        "signal": "succeeded"
      }
    },
    {
      "operation": "connect",
      "route": {
        "id": "audit-finish",
        "source_node_id": "audit",
        "destination_node_id": "finish",
        "signal": "passed"
      }
    }
  ]
}
```

```powershell
gareji-board control-graph apply --file control-graph-edit.json
```

Supported operations are `add_node`, `remove_node`, `connect`, and `remove_route`.
Control node kinds are `agent_loop`, `gate`, `audit`, `approval`, and `terminal`.
An `agent_loop` kind also requires `agent_profile_id`.

## Publish a Blueprint edit plan

Blueprint plans use the same source/new revision fields and the operations `add_node`,
`remove_node`, `connect`, and `remove_link`. A node contains its complete typed
`inputs` and `outputs`; a connection contains either a `flow` signal or `data` socket.

```powershell
gareji-board blueprint apply --file blueprint-edit.json
Get-Content blueprint-edit.json | gareji-board blueprint apply --file -
```

The complete plan is applied in memory. Board writes the new revision only after all
nodes are reachable and all signals, sockets, and authority rules validate.
