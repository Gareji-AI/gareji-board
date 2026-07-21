use std::path::Path;

use gareji_board_domain::{PortfolioRun, PortfolioRunStatus};
use gareji_board_store::{SqliteBoardStore, StoreError};
use serde::Serialize;
use thiserror::Error;

use crate::{
    PortfolioPreviewFacts, PortfolioRunController, PortfolioTickError, PortfolioTickMode,
    PortfolioTickRequest,
};

/// One durable tick recorded by a scheduler pass.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduledPortfolioTick {
    pub orchestration_id: String,
    pub revision_id: String,
    pub run_id: String,
    pub node_id: String,
    pub sequence: u32,
}

/// Bounded summary returned after inspecting all immutable Portfolio revisions once.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PortfolioSchedulerReport {
    pub inspected_revisions: usize,
    pub recorded_ticks: Vec<ScheduledPortfolioTick>,
    pub contended_revisions: usize,
}

/// A one-pass scheduler failed before it could safely finish the portfolio scan.
#[derive(Debug, Error)]
pub enum PortfolioSchedulerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Portfolio tick failed for {orchestration_id}/{revision_id}: {source}")]
    Tick {
        orchestration_id: String,
        revision_id: String,
        #[source]
        source: PortfolioTickError,
    },
}

/// Deep Module for advancing every due Portfolio revision at most once.
///
/// The caller supplies one wall-clock value and gets a finite report. Process
/// lifetime, polling, and OS task registration deliberately stay outside this
/// Module, so it can be used by both the desktop and one-shot local automation.
pub struct PortfolioScheduler {
    store: SqliteBoardStore,
}

impl PortfolioScheduler {
    /// Open Board-owned state from its local `SQLite` file.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the Board database cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortfolioSchedulerError> {
        Ok(Self::new(SqliteBoardStore::open(path)?))
    }

    /// Compose the Module with a local-substitutable store.
    #[must_use]
    pub const fn new(store: SqliteBoardStore) -> Self {
        Self { store }
    }

    /// Inspect all stored revisions and durably advance each due revision once.
    ///
    /// Revisions with manual, disabled, paused, waiting-approval, or future
    /// schedules are skipped. A competing desktop or scheduler process is
    /// reported as contention instead of duplicating a tick.
    ///
    /// # Errors
    ///
    /// Returns a storage or fail-closed Portfolio tick error.
    pub fn tick_due_once(
        &mut self,
        now_epoch_seconds: i64,
    ) -> Result<PortfolioSchedulerReport, PortfolioSchedulerError> {
        let portfolio = self.store.load_portfolio()?;
        let work_items = self.store.load_work_items()?;
        let agent_profiles = self.store.load_agent_profiles()?;
        let project_graph_bindings = self.store.load_project_graph_bindings()?;
        let control_graphs = self.store.load_control_graph_revisions()?;
        let revisions = self.store.load_portfolio_orchestration_revisions()?;
        let facts = PortfolioPreviewFacts {
            portfolio: &portfolio,
            work_items: &work_items,
            agent_profiles: &agent_profiles,
            project_graph_bindings: &project_graph_bindings,
            control_graphs: &control_graphs,
        };
        let mut report = PortfolioSchedulerReport {
            inspected_revisions: revisions.len(),
            ..PortfolioSchedulerReport::default()
        };

        for revision in &revisions {
            if revision.schedule.interval_seconds().is_none()
                || !self
                    .store
                    .load_portfolio_schedule_control(revision)?
                    .automatic_ticks_enabled
            {
                continue;
            }
            let observed_latest = self
                .store
                .load_latest_portfolio_run(&revision.orchestration_id, &revision.revision_id)?;
            let Some(due_run) = due_current_run(observed_latest.as_ref(), now_epoch_seconds) else {
                continue;
            };
            let run_id = scheduled_run_id(revision, now_epoch_seconds);
            let receipt = PortfolioRunController::tick(&PortfolioTickRequest {
                revision,
                current_run: due_run.current_run(),
                run_id: &run_id,
                now_epoch_seconds,
                mode: PortfolioTickMode::DueOnly,
                facts: &facts,
            })
            .map_err(|source| PortfolioSchedulerError::Tick {
                orchestration_id: revision.orchestration_id.clone(),
                revision_id: revision.revision_id.clone(),
                source,
            })?;
            match self.store.record_portfolio_tick(
                observed_latest.as_ref(),
                &receipt.resulting,
                &receipt.step,
            ) {
                Ok(()) => report.recorded_ticks.push(ScheduledPortfolioTick {
                    orchestration_id: revision.orchestration_id.clone(),
                    revision_id: revision.revision_id.clone(),
                    run_id: receipt.resulting.run_id,
                    node_id: receipt.step.node_id,
                    sequence: receipt.step.sequence,
                }),
                Err(StoreError::ConcurrentChange) => report.contended_revisions += 1,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(report)
    }
}

enum DuePortfolioRun<'a> {
    Start,
    Continue(&'a PortfolioRun),
}

impl DuePortfolioRun<'_> {
    const fn current_run(&self) -> Option<&PortfolioRun> {
        match self {
            Self::Start => None,
            Self::Continue(run) => Some(run),
        }
    }
}

fn due_current_run(
    latest: Option<&PortfolioRun>,
    now_epoch_seconds: i64,
) -> Option<DuePortfolioRun<'_>> {
    match latest {
        Some(run) if run.status == PortfolioRunStatus::Active && run.is_due(now_epoch_seconds) => {
            Some(DuePortfolioRun::Continue(run))
        }
        Some(run)
            if run.status == PortfolioRunStatus::Completed
                && run
                    .next_tick_at_epoch_seconds
                    .is_some_and(|due| due <= now_epoch_seconds) =>
        {
            Some(DuePortfolioRun::Start)
        }
        None => Some(DuePortfolioRun::Start),
        Some(_) => None,
    }
}

fn scheduled_run_id(
    revision: &gareji_board_domain::PortfolioOrchestrationRevision,
    now_epoch_seconds: i64,
) -> String {
    format!(
        "portfolio-run-{}-{}-{now_epoch_seconds}",
        revision.orchestration_id, revision.revision_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pass_records_only_due_revisions() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store.ensure_builtin_portfolio_orchestrations().unwrap();
        let mut scheduler = PortfolioScheduler::new(store);

        let first = scheduler.tick_due_once(1_000).unwrap();
        assert_eq!(first.inspected_revisions, 1);
        assert_eq!(first.recorded_ticks.len(), 1);
        assert_eq!(first.recorded_ticks[0].node_id, "select-project");

        let early_retry = scheduler.tick_due_once(1_001).unwrap();
        assert!(early_retry.recorded_ticks.is_empty());

        let next_due = scheduler.tick_due_once(4_600).unwrap();
        assert_eq!(next_due.recorded_ticks.len(), 1);
        assert_eq!(next_due.recorded_ticks[0].node_id, "summary");
        assert_eq!(next_due.recorded_ticks[0].sequence, 2);
    }
}
