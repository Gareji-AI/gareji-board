# Board bridge v0

`gareji-board bridge` is the bounded local Interface through which Core asks Board one question: may this Work item be associated with current active work?

The protocol discriminator is `gareji.board-bridge.v0`. Each newline-delimited JSON request is limited to 1 MiB and contains a request identity plus exactly one operation:

```text
assess_active_work(project_id, work_item_id)
```

The response preserves project identity, Work item identity, current Board-owned state, and either `eligible` or a stable ineligible reason. Unknown or unrelated Work items both return `work_item_not_found`, preventing a caller from discovering an item through the wrong project.

The Bridge does not select a Runner candidate, change Work item state, store Core active-work references, execute code, or access another project's records. Closing the parent pipe ends the child process; no background daemon is installed.

For the bundled sample:

```text
gareji-board seed-sample
gareji-board bridge
```

The desktop app and Bridge resolve the same Board SQLite location. `GAREJI_BOARD_DB` or `--database` may explicitly select another local file for tests and disposable demos.
