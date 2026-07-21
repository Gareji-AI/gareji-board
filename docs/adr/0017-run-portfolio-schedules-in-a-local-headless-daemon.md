# ADR 0017: Run Portfolio schedules in a local headless daemon

## Status

Accepted

## Context

ADR 0015 used repeating one-shot OS tasks so Portfolio schedules could continue while
the desktop was closed. That remains safe, but it gives the OS task cadence ownership
and makes short polling intervals cumbersome. Local operators need one explicit
headless process that can stay alive without opening the Tauri window.

The scheduler must keep Board's existing authority rules. It must not start a Project
Runner, bypass an Approval node, create a remote scheduler, or duplicate a tick when
the desktop and headless process overlap.

## Decision

Board exposes `gareji-board portfolio-daemon`. The command opens the same Board SQLite
database as the desktop and repeatedly calls the existing finite `PortfolioScheduler`
Module with one observed wall-clock value per pass. The polling interval controls only
how often due work is checked; immutable Portfolio revisions continue to own their
actual schedules.

`gareji-board portfolio-tick-due` remains available for diagnostics, cron, launchd,
and other one-shot integrations. The daemon is a local CLI Adapter and does not move
process lifetime into the scheduler Module.

On Windows, the installation script registers the daemon as a hidden current-user
Task Scheduler process triggered at logon and starts it immediately. The task has no
execution time limit and is restarted after a failed process. It is not a machine-wide
Windows service and does not run while no user is logged in.

Every pass retains ADR 0015's immediate SQLite transaction and exact latest-Run
comparison. A competing desktop or daemon process loses with contention rather than
recording a duplicate tick. Waiting approvals remain paused for a person.

## Consequences

- Portfolio cadence continues while the Tauri UI is closed.
- GUI, one-shot CLI, and daemon share one finite Scheduler Module and transaction rule.
- A fatal storage or validation error terminates the daemon so the OS task can restart
  it instead of silently looping in a broken state.
- Current-user logon is still required; machine services and remote scheduling remain
  out of scope.
