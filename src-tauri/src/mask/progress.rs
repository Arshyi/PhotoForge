use crate::error::AppError;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub(crate) const IO_PROGRESS_CHUNK_PIXELS: u64 = 64 * 1024;

pub type SharedMaskProgress = Arc<Mutex<Option<MaskProgress>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskProgressState {
    Queued,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

impl MaskProgressState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskProgress {
    pub document_id: u64,
    pub request_id: u64,
    pub operation: String,
    pub phase: String,
    pub completed_units: u64,
    pub total_units: u64,
    pub state: MaskProgressState,
}

impl MaskProgress {
    fn queued(document_id: u64, request_id: u64, operation: String) -> Self {
        Self {
            document_id,
            request_id,
            operation,
            phase: "queued".into(),
            completed_units: 0,
            total_units: 0,
            state: MaskProgressState::Queued,
        }
    }
}

#[derive(Clone)]
pub struct MaskProgressHandle {
    shared: SharedMaskProgress,
    document_id: u64,
    request_id: u64,
}

impl MaskProgressHandle {
    pub fn begin(
        shared: SharedMaskProgress,
        document_id: u64,
        request_id: u64,
        operation: impl Into<String>,
    ) -> Result<Self, AppError> {
        *lock_progress(&shared)? = Some(MaskProgress::queued(
            document_id,
            request_id,
            operation.into(),
        ));
        Ok(Self {
            shared,
            document_id,
            request_id,
        })
    }

    pub fn report(
        &self,
        phase: &str,
        completed_units: u64,
        total_units: u64,
    ) -> Result<(), AppError> {
        self.update(|progress| {
            if progress.state.is_terminal() {
                return;
            }
            progress.phase = phase.to_owned();
            progress.total_units = progress.total_units.max(total_units);
            progress.completed_units = progress
                .completed_units
                .max(completed_units.min(progress.total_units));
            if progress.state != MaskProgressState::Cancelling {
                progress.state = MaskProgressState::Running;
            }
        })
    }

    pub fn mark_running(&self, phase: &str) -> Result<(), AppError> {
        self.update(|progress| {
            if !progress.state.is_terminal() && progress.state != MaskProgressState::Cancelling {
                progress.phase = phase.to_owned();
                progress.state = MaskProgressState::Running;
            }
        })
    }

    pub fn request_cancel(&self) -> Result<bool, AppError> {
        let mut changed = false;
        self.update(|progress| {
            if !progress.state.is_terminal() {
                progress.state = MaskProgressState::Cancelling;
                progress.phase = "cancelling".into();
                changed = true;
            }
        })?;
        Ok(changed)
    }

    pub fn is_cancelling(&self) -> Result<bool, AppError> {
        let progress = lock_progress(&self.shared)?;
        Ok(progress.as_ref().is_some_and(|progress| {
            self.matches(progress) && progress.state == MaskProgressState::Cancelling
        }))
    }

    pub fn complete(&self) -> Result<(), AppError> {
        self.terminal(MaskProgressState::Completed, "completed", true)
    }

    pub fn acknowledge_cancelled(&self) -> Result<(), AppError> {
        self.terminal(MaskProgressState::Cancelled, "cancelled", false)
    }

    pub fn fail(&self) -> Result<(), AppError> {
        self.terminal(MaskProgressState::Failed, "failed", false)
    }

    pub fn snapshot(&self) -> Result<Option<MaskProgress>, AppError> {
        snapshot(&self.shared, self.document_id, self.request_id)
    }

    pub fn planned(&self, total_units: u64) -> PlannedMaskProgress {
        PlannedMaskProgress {
            handle: self.clone(),
            total_units,
            phase: Arc::new(Mutex::new(PlannedPhase::default())),
        }
    }

    fn terminal(
        &self,
        state: MaskProgressState,
        phase: &str,
        complete_units: bool,
    ) -> Result<(), AppError> {
        self.update(|progress| {
            if state == MaskProgressState::Completed
                && progress.state == MaskProgressState::Cancelling
            {
                return;
            }
            if complete_units {
                progress.completed_units = progress.total_units;
            }
            progress.phase = phase.to_owned();
            progress.state = state;
        })
    }

    fn update(&self, update: impl FnOnce(&mut MaskProgress)) -> Result<(), AppError> {
        let mut shared = lock_progress(&self.shared)?;
        if let Some(progress) = shared.as_mut().filter(|progress| self.matches(progress)) {
            update(progress);
            progress.completed_units = progress.completed_units.min(progress.total_units);
        }
        Ok(())
    }

    fn matches(&self, progress: &MaskProgress) -> bool {
        progress.document_id == self.document_id && progress.request_id == self.request_id
    }
}

#[derive(Default)]
struct PlannedPhase {
    name: String,
    offset: u64,
    total_units: u64,
}

#[derive(Clone)]
pub struct PlannedMaskProgress {
    handle: MaskProgressHandle,
    total_units: u64,
    phase: Arc<Mutex<PlannedPhase>>,
}

impl PlannedMaskProgress {
    pub fn report_local(
        &self,
        phase: &str,
        completed_units: u64,
        phase_total_units: u64,
    ) -> Result<(), AppError> {
        let (completed, required_total) = {
            let mut current = self.phase.lock().map_err(|_| {
                AppError::ProcessingFailure("mask progress plan is unavailable".into())
            })?;
            if current.name != phase {
                current.offset = current.offset.saturating_add(current.total_units);
                current.name = phase.to_owned();
                current.total_units = phase_total_units;
            } else {
                current.total_units = current.total_units.max(phase_total_units);
            }
            (
                current
                    .offset
                    .saturating_add(completed_units.min(current.total_units)),
                current.offset.saturating_add(current.total_units),
            )
        };
        self.handle
            .report(phase, completed, self.total_units.max(required_total))
    }
}

pub fn snapshot(
    shared: &SharedMaskProgress,
    document_id: u64,
    request_id: u64,
) -> Result<Option<MaskProgress>, AppError> {
    Ok(lock_progress(shared)?
        .as_ref()
        .filter(|progress| progress.document_id == document_id && progress.request_id == request_id)
        .cloned())
}

pub fn request_cancel(shared: &SharedMaskProgress, request_id: u64) -> Result<bool, AppError> {
    let mut shared = lock_progress(shared)?;
    let Some(progress) = shared
        .as_mut()
        .filter(|progress| progress.request_id == request_id && !progress.state.is_terminal())
    else {
        return Ok(false);
    };
    progress.state = MaskProgressState::Cancelling;
    progress.phase = "cancelling".into();
    Ok(true)
}

fn lock_progress(
    shared: &SharedMaskProgress,
) -> Result<MutexGuard<'_, Option<MaskProgress>>, AppError> {
    shared
        .lock()
        .map_err(|_| AppError::ProcessingFailure("mask progress state is unavailable".into()))
}

pub type MaskProgressCallback<'a> =
    dyn Fn(&str, u64, u64) -> Result<(), AppError> + Send + Sync + 'a;

#[derive(Clone, Copy)]
pub struct MaskWorkContext<'a> {
    cancelled: Option<&'a AtomicBool>,
    progress: Option<&'a MaskProgressCallback<'a>>,
}

impl<'a> MaskWorkContext<'a> {
    pub fn new(
        cancelled: Option<&'a AtomicBool>,
        progress: Option<&'a MaskProgressCallback<'a>>,
    ) -> Self {
        Self {
            cancelled,
            progress,
        }
    }

    pub fn cancellation_only(cancelled: Option<&'a AtomicBool>) -> Self {
        Self::new(cancelled, None)
    }

    pub fn check_cancelled(self) -> Result<(), AppError> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            Err(AppError::MaskCancelled)
        } else {
            Ok(())
        }
    }

    pub fn report(
        self,
        phase: &str,
        completed_units: u64,
        total_units: u64,
    ) -> Result<(), AppError> {
        self.check_cancelled()?;
        if let Some(progress) = self.progress {
            progress(phase, completed_units.min(total_units), total_units)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedMaskProgress {
        Arc::new(Mutex::new(None))
    }

    #[test]
    fn reports_are_monotonic_and_clamped() {
        let handle = MaskProgressHandle::begin(shared(), 4, 7, "feather").unwrap();
        let plan = handle.planned(15);
        plan.report_local("horizontal", 6, 10).unwrap();
        plan.report_local("horizontal", 3, 10).unwrap();
        assert_eq!(handle.snapshot().unwrap().unwrap().completed_units, 6);
        plan.report_local("horizontal", 99, 10).unwrap();
        let progress = handle.snapshot().unwrap().unwrap();
        assert_eq!(progress.completed_units, 10);
        assert_eq!(progress.total_units, 15);

        plan.report_local("vertical", 2, 5).unwrap();
        let progress = handle.snapshot().unwrap().unwrap();
        assert_eq!(progress.completed_units, 12);
        assert_eq!(progress.total_units, 15);
        assert!(progress.completed_units <= progress.total_units);
    }

    #[test]
    fn snapshots_are_filtered_and_stale_handles_cannot_overwrite() {
        let shared = shared();
        let stale = MaskProgressHandle::begin(shared.clone(), 1, 1, "wand").unwrap();
        let current = MaskProgressHandle::begin(shared.clone(), 2, 2, "range").unwrap();
        stale.complete().unwrap();
        assert!(snapshot(&shared, 1, 1).unwrap().is_none());
        assert_eq!(
            current.snapshot().unwrap().unwrap().state,
            MaskProgressState::Queued
        );
    }

    #[test]
    fn terminal_and_cancellation_states_are_explicit() {
        let completed = MaskProgressHandle::begin(shared(), 1, 1, "compose").unwrap();
        completed.report("pixels", 2, 4).unwrap();
        completed.complete().unwrap();
        let progress = completed.snapshot().unwrap().unwrap();
        assert_eq!(progress.state, MaskProgressState::Completed);
        assert_eq!(progress.completed_units, progress.total_units);

        let failed = MaskProgressHandle::begin(shared(), 1, 2, "compose").unwrap();
        failed.fail().unwrap();
        assert_eq!(
            failed.snapshot().unwrap().unwrap().state,
            MaskProgressState::Failed
        );

        let cancelled = MaskProgressHandle::begin(shared(), 1, 3, "compose").unwrap();
        assert!(cancelled.request_cancel().unwrap());
        assert_eq!(
            cancelled.snapshot().unwrap().unwrap().state,
            MaskProgressState::Cancelling
        );
        cancelled.complete().unwrap();
        assert_eq!(
            cancelled.snapshot().unwrap().unwrap().state,
            MaskProgressState::Cancelling
        );
        cancelled.acknowledge_cancelled().unwrap();
        assert_eq!(
            cancelled.snapshot().unwrap().unwrap().state,
            MaskProgressState::Cancelled
        );
    }

    #[test]
    fn indeterminate_zero_unit_work_completes_without_inventing_units() {
        let handle = MaskProgressHandle::begin(shared(), 9, 10, "parse_mask_file").unwrap();
        handle.report("parse_mask_file", 42, 0).unwrap();
        let running = handle.snapshot().unwrap().unwrap();
        assert_eq!(running.completed_units, 0);
        assert_eq!(running.total_units, 0);
        assert_eq!(running.state, MaskProgressState::Running);

        handle.complete().unwrap();
        let completed = handle.snapshot().unwrap().unwrap();
        assert_eq!(completed.completed_units, 0);
        assert_eq!(completed.total_units, 0);
        assert_eq!(completed.state, MaskProgressState::Completed);
    }
}
