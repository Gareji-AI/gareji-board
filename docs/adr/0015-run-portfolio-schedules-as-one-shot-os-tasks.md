# ADR 0015: Run Portfolio schedules as one-shot OS tasks

## Status

Accepted

## Context

Portfolio orchestration revisions can declare an interval and the desktop can service
that cadence while it is open. Scheduling must also work while the desktop is closed,
without turning Board into a daemon or moving graph, schedule, or approval authority
into a remote service.

The desktop and an external wake-up can overlap. A completed interval Run can also be
restarted, so checking only the resulting Run identifier is not enough to prevent two
processes from creating parallel successor Runs.

## Decision

Board exposes `gareji-board portfolio-tick-due` as a finite local Scheduler Adapter.
One invocation opens the configured Board SQLite database, inspects every immutable
Portfolio revision, advances each due revision at most once, prints a bounded JSON
report, and exits. The desktop uses the same `PortfolioScheduler` Module rather than
maintaining a second implementation.

Windows Task Scheduler, launchd, cron, or an equivalent local facility owns wake-up
and process lifetime. The Windows setup script builds the existing Board CLI and
registers a repeating task for the current user. Board does not add a daemon.

Every tick write starts an immediate SQLite transaction and compares the latest Run
for the orchestration and revision with the exact Run observed by the caller. This
comparison covers both an ordinary advance and creation of a successor to a completed
Run. A losing process reports contention and does not duplicate the step.

The Scheduler Adapter never starts a project Runner. Waiting approvals remain paused
until a person records an explicit decision through Board.

## Consequences

- Portfolio cadence continues while the desktop window is closed and the user session
  can run its configured OS task.
- GUI and scheduled execution share one Module and one transaction rule.
- Missed wake-ups can be serviced by the next OS invocation with `StartWhenAvailable`.
- Machine-wide service installation, execution while no user is logged in, and remote
  scheduling remain out of scope.
