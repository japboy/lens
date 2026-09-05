//! Finite observation policy; physical clocks, tasks and native registrations stay in desktop.
use crate::{
    model::{LensFreshness, LensMonitoringLifecycle, LensSourceHealth, LensState},
    state::LensContextRefreshOutcome,
};
use port_platform::observation::WindowObservationEvent;
use std::{collections::BTreeMap, num::NonZeroU64, time::Duration};
use uuid::Uuid;

pub const OBSERVATION_COALESCING_INTERVAL: Duration = Duration::from_millis(250);
pub const PERIODIC_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum LiveSignal {
    Invalidation(WindowObservationEvent),
    ImmediateRefresh,
    PeriodicReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationStart {
    Initial,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSubmissionDirective {
    None,
    LiveProjectionUpdate,
    RecoveryCheckpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshSchedulingState {
    recovery_checkpoint_pending: bool,
}

impl RefreshSchedulingState {
    pub fn new(start: ObservationStart) -> Self {
        Self {
            recovery_checkpoint_pending: start == ObservationStart::Resume,
        }
    }

    pub fn completed_refresh(
        &mut self,
        outcome: LensContextRefreshOutcome,
    ) -> AgentSubmissionDirective {
        if self.recovery_checkpoint_pending {
            self.recovery_checkpoint_pending = false;
            AgentSubmissionDirective::RecoveryCheckpoint
        } else if outcome == LensContextRefreshOutcome::Updated {
            AgentSubmissionDirective::LiveProjectionUpdate
        } else {
            AgentSubmissionDirective::None
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObservationCoverage {
    pub expected_sources: usize,
    pub observing_sources: usize,
    pub has_registration_diagnostics: bool,
    pub failures: Vec<String>,
}

impl ObservationCoverage {
    pub fn is_usable(&self) -> bool {
        self.expected_sources > 0
    }

    pub fn has_missing_sources(&self) -> bool {
        self.observing_sources != self.expected_sources
    }

    pub fn message(&self) -> Option<String> {
        (!self.failures.is_empty()).then(|| self.failures.join("; "))
    }
}

pub fn valid_signal(
    signal: &LiveSignal,
    operation_id: Uuid,
    context_id: Uuid,
    observer_epoch: NonZeroU64,
    authority: &BTreeMap<Uuid, u32>,
) -> bool {
    match signal {
        LiveSignal::ImmediateRefresh | LiveSignal::PeriodicReconciliation => true,
        LiveSignal::Invalidation(event) => {
            event.operation_id == operation_id
                && event.context_id == context_id
                && event.observer_epoch == observer_epoch
                && authority.get(&event.source_registration_id) == Some(&event.window_id)
        }
    }
}

impl LiveSignal {
    pub fn is_immediate(&self) -> bool {
        matches!(self, Self::ImmediateRefresh | Self::PeriodicReconciliation)
    }
}

impl ObservationCoverage {
    pub fn apply_to(&self, lens: &mut LensState) {
        let message = self.message();
        let Some(live) = lens.live.as_mut() else {
            return;
        };
        if self.has_missing_sources() {
            live.health = LensSourceHealth::Unavailable;
            live.freshness = LensFreshness::Unverified;
            live.error = message;
        } else if self.has_registration_diagnostics && live.health == LensSourceHealth::Healthy {
            live.health = LensSourceHealth::Degraded;
        }
    }
}

pub fn refresh_revision(lens: &LensState, operation_id: Uuid, context_id: Uuid) -> Option<u64> {
    if lens.operation_id != Some(operation_id)
        || !lens
            .live
            .as_ref()
            .is_some_and(|live| live.lifecycle == LensMonitoringLifecycle::Watching)
    {
        return None;
    }
    lens.context
        .as_ref()
        .and_then(|context| (context.context_id == context_id).then_some(context.revision))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidation_requires_every_source_authority_axis() {
        use port_platform::observation::WindowObservationNotification;
        let event = WindowObservationEvent {
            operation_id: Uuid::from_u128(1),
            context_id: Uuid::from_u128(2),
            source_registration_id: Uuid::from_u128(3),
            observer_epoch: NonZeroU64::new(1).unwrap(),
            window_id: 42,
            notification: WindowObservationNotification::WindowTitleChanged,
        };
        let authority = BTreeMap::from([(event.source_registration_id, event.window_id)]);
        let admits = |signal: &LiveSignal| {
            valid_signal(
                signal,
                event.operation_id,
                event.context_id,
                event.observer_epoch,
                &authority,
            )
        };
        assert!(admits(&LiveSignal::Invalidation(event.clone())));
        assert!(!LiveSignal::Invalidation(event.clone()).is_immediate());
        for signal in [
            LiveSignal::ImmediateRefresh,
            LiveSignal::PeriodicReconciliation,
        ] {
            assert!(signal.is_immediate());
            assert!(admits(&signal));
        }
        for axis in 0..5 {
            let mut stale = event.clone();
            match axis {
                0 => stale.operation_id = Uuid::from_u128(4),
                1 => stale.context_id = Uuid::from_u128(4),
                2 => stale.source_registration_id = Uuid::from_u128(4),
                3 => stale.observer_epoch = NonZeroU64::new(2).unwrap(),
                4 => stale.window_id = 43,
                _ => unreachable!(),
            }
            assert!(!admits(&LiveSignal::Invalidation(stale)));
        }
    }

    #[test]
    fn polling_fallback_remains_usable_when_every_observer_registration_failed() {
        let coverage = ObservationCoverage {
            expected_sources: 2,
            observing_sources: 0,
            has_registration_diagnostics: false,
            failures: vec!["source unavailable".into()],
        };

        assert!(coverage.is_usable());
        assert!(coverage.has_missing_sources());
        assert_eq!(PERIODIC_RECONCILIATION_INTERVAL, Duration::from_secs(30));
    }

    #[test]
    fn periodic_reconciliation_is_authoritative_without_observer_callbacks() {
        assert!(valid_signal(
            &LiveSignal::PeriodicReconciliation,
            Uuid::nil(),
            Uuid::nil(),
            NonZeroU64::new(1).expect("epoch"),
            &BTreeMap::new(),
        ));
    }

    #[test]
    fn resume_requires_one_recovery_checkpoint_even_for_an_unchanged_projection() {
        let mut initial = RefreshSchedulingState::new(ObservationStart::Initial);
        assert_eq!(
            initial.completed_refresh(LensContextRefreshOutcome::Unchanged),
            AgentSubmissionDirective::None
        );
        assert_eq!(
            initial.completed_refresh(LensContextRefreshOutcome::Updated),
            AgentSubmissionDirective::LiveProjectionUpdate
        );

        for outcome in [
            LensContextRefreshOutcome::Unchanged,
            LensContextRefreshOutcome::Updated,
        ] {
            let mut resumed = RefreshSchedulingState::new(ObservationStart::Resume);
            assert_eq!(
                resumed.completed_refresh(outcome),
                AgentSubmissionDirective::RecoveryCheckpoint
            );
            assert_eq!(
                resumed.completed_refresh(LensContextRefreshOutcome::Unchanged),
                AgentSubmissionDirective::None
            );
        }
    }
}
