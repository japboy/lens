use crate::{
    agent,
    app_state::{update_lens_state, AppState, LensContextRefreshOutcome},
    commands::refresh_lens_context,
    lens::LensTargetSet,
    model::{LensFreshness, LensMonitoringLifecycle, LensSourceHealth, LensStage, LensState},
    platform::{
        self, WindowObservationEvent, WindowObservationReceiver, WindowObservationRegistration,
    },
};
use std::{collections::BTreeMap, num::NonZeroU64, sync::Mutex, time::Duration};
use tauri::{async_runtime::JoinHandle, AppHandle, Manager};
use tokio::{sync::mpsc, time::MissedTickBehavior};
use uuid::Uuid;

const OBSERVATION_COALESCING_INTERVAL: Duration = Duration::from_millis(250);
const PERIODIC_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct LensLiveControl {
    inner: Mutex<LiveControlState>,
}

#[derive(Default)]
struct LiveControlState {
    operation: Option<LiveOperation>,
}

struct LiveOperation {
    operation_id: Uuid,
    context_id: Uuid,
    observer_epoch: NonZeroU64,
    active: Option<ActiveObservation>,
}

struct ActiveObservation {
    signal: mpsc::Sender<LiveSignal>,
    scheduler: JoinHandle<()>,
    reconciler: JoinHandle<()>,
    forwarders: Vec<JoinHandle<()>>,
    registrations: Vec<WindowObservationRegistration>,
}

impl ActiveObservation {
    fn shutdown(mut self) {
        self.scheduler.abort();
        self.reconciler.abort();
        for forwarder in self.forwarders.drain(..) {
            forwarder.abort();
        }
        // Explicit close reports teardown failure; the adapter's Drop remains a cleanup backstop.
        for registration in &mut self.registrations {
            if let Err(error) = registration.close() {
                eprintln!("Lens observer close failed: {error}");
            }
        }
        self.registrations.clear();
    }
}

#[derive(Debug)]
enum LiveSignal {
    Invalidation(WindowObservationEvent),
    ImmediateRefresh,
    PeriodicReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationStart {
    Initial,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentSubmissionDirective {
    None,
    LiveProjectionUpdate,
    RecoveryCheckpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RefreshSchedulingState {
    recovery_checkpoint_pending: bool,
}

impl RefreshSchedulingState {
    fn new(start: ObservationStart) -> Self {
        Self {
            recovery_checkpoint_pending: start == ObservationStart::Resume,
        }
    }

    fn completed_refresh(
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
struct ObservationCoverage {
    expected_sources: usize,
    observing_sources: usize,
    has_registration_diagnostics: bool,
    failures: Vec<String>,
}

#[derive(Debug)]
struct ObservationSchedulerContext {
    operation_id: Uuid,
    context_id: Uuid,
    observer_epoch: NonZeroU64,
    source_window_authority: BTreeMap<Uuid, u32>,
    coverage: ObservationCoverage,
    start: ObservationStart,
}

impl ObservationCoverage {
    fn is_usable(&self) -> bool {
        self.expected_sources > 0
    }

    fn has_missing_sources(&self) -> bool {
        self.observing_sources != self.expected_sources
    }

    fn message(&self) -> Option<String> {
        (!self.failures.is_empty()).then(|| self.failures.join("; "))
    }
}

struct ObservationSetup {
    active: Option<ActiveObservation>,
    coverage: ObservationCoverage,
}

impl LensLiveControl {
    fn install(
        &self,
        app: AppHandle,
        operation_id: Uuid,
        context_id: Uuid,
        target_set: &LensTargetSet,
        start: ObservationStart,
    ) -> Result<ObservationCoverage, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Lens live control lock is poisoned".to_string())?;

        let observer_epoch = match inner.operation.as_mut() {
            Some(operation)
                if operation.operation_id == operation_id && operation.context_id == context_id =>
            {
                if operation.active.is_some() {
                    return Err("Lens monitoring is already active".into());
                }
                if start != ObservationStart::Resume {
                    return Err("Lens monitoring was already initialized for this operation".into());
                }
                let next = operation
                    .observer_epoch
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU64::new)
                    .ok_or_else(|| "Lens observer epoch is exhausted".to_string())?;
                operation.observer_epoch = next;
                next
            }
            Some(_) => {
                return Err("another Lens monitoring operation is still registered".into());
            }
            None if start == ObservationStart::Resume => {
                return Err("the paused Lens monitoring operation is unavailable".into());
            }
            None => NonZeroU64::new(1).expect("initial observer epoch is non-zero"),
        };

        let setup = build_observation(
            app,
            operation_id,
            context_id,
            observer_epoch,
            target_set,
            start,
        );
        let coverage = setup.coverage.clone();
        match inner.operation.as_mut() {
            Some(operation) => operation.active = setup.active,
            None => {
                inner.operation = Some(LiveOperation {
                    operation_id,
                    context_id,
                    observer_epoch,
                    active: setup.active,
                });
            }
        }
        Ok(coverage)
    }

    fn pause(&self, operation_id: Uuid) -> Result<(), String> {
        let active = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "Lens live control lock is poisoned".to_string())?;
            let operation = inner
                .operation
                .as_mut()
                .filter(|operation| operation.operation_id == operation_id)
                .ok_or_else(|| "Lens monitoring operation is unavailable".to_string())?;
            operation.active.take()
        };
        if let Some(active) = active {
            active.shutdown();
        }
        Ok(())
    }

    fn stop(&self, operation_id: Uuid) -> Result<(), String> {
        let operation = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "Lens live control lock is poisoned".to_string())?;
            let Some(operation) = inner.operation.as_ref() else {
                return Ok(());
            };
            if operation.operation_id != operation_id {
                return Err("Lens monitoring operation was superseded".into());
            }
            inner.operation.take()
        };
        if let Some(active) = operation.and_then(|operation| operation.active) {
            active.shutdown();
        }
        Ok(())
    }

    fn request_immediate_refresh(&self, operation_id: Uuid) -> Result<(), String> {
        let signal = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| "Lens live control lock is poisoned".to_string())?;
            inner
                .operation
                .as_ref()
                .filter(|operation| operation.operation_id == operation_id)
                .and_then(|operation| operation.active.as_ref())
                .map(|active| active.signal.clone())
                .ok_or_else(|| "Lens monitoring is not active".to_string())?
        };
        match signal.try_send(LiveSignal::ImmediateRefresh) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err("Lens monitoring scheduler is unavailable".into())
            }
        }
    }
}

pub fn start(app: &AppHandle, operation_id: Uuid) -> Result<(), String> {
    let lens = require_live_operation(app, operation_id, LensMonitoringLifecycle::Watching)?;
    let context = lens
        .context
        .as_ref()
        .ok_or_else(|| "the active Lens operation has no canonical context".to_string())?;
    let target_set = lens
        .target_set
        .as_ref()
        .ok_or_else(|| "the active Lens operation has no fixed target set".to_string())?;
    let coverage = app.state::<AppState>().live_control.install(
        app.clone(),
        operation_id,
        context.context_id,
        target_set,
        ObservationStart::Initial,
    )?;
    apply_observation_coverage(app, operation_id, &coverage)?;
    Ok(())
}

pub fn request_immediate_refresh(app: &AppHandle, operation_id: Uuid) -> Result<(), String> {
    app.state::<AppState>()
        .live_control
        .request_immediate_refresh(operation_id)
}

pub fn pause(app: &AppHandle, operation_id: Uuid) -> Result<LensState, String> {
    require_live_operation(app, operation_id, LensMonitoringLifecycle::Watching)?;
    if !update_lens_state(app, operation_id, |lens| {
        let Some(live) = lens.live.as_mut() else {
            return;
        };
        live.lifecycle = LensMonitoringLifecycle::Paused;
        live.freshness = LensFreshness::Unverified;
        live.error = None;
        lens.pending_representation = None;
        if lens.representation.is_some() {
            lens.stage = LensStage::Completed;
        } else {
            lens.stage = LensStage::Ready;
        }
    })? {
        return Err("Lens operation was superseded before monitoring paused".into());
    }
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    state.live_control.pause(operation_id)?;
    state.lens()
}

pub fn resume(app: &AppHandle, operation_id: Uuid) -> Result<LensState, String> {
    let lens = require_live_operation(app, operation_id, LensMonitoringLifecycle::Paused)?;
    let context = lens
        .context
        .as_ref()
        .ok_or_else(|| "the paused Lens operation has no canonical context".to_string())?;
    let target_set = lens
        .target_set
        .as_ref()
        .ok_or_else(|| "the paused Lens operation has no fixed target set".to_string())?;
    let coverage = app.state::<AppState>().live_control.install(
        app.clone(),
        operation_id,
        context.context_id,
        target_set,
        ObservationStart::Resume,
    )?;
    if !coverage.is_usable() {
        apply_observation_coverage(app, operation_id, &coverage)?;
        return Err(coverage
            .message()
            .unwrap_or_else(|| "none of the fixed Lens targets could be observed".into()));
    }
    if !update_lens_state(app, operation_id, |lens| {
        let Some(live) = lens.live.as_mut() else {
            return;
        };
        live.lifecycle = LensMonitoringLifecycle::Watching;
        live.freshness = if coverage.has_missing_sources() {
            LensFreshness::Unverified
        } else {
            LensFreshness::Checking
        };
        live.last_outcome = None;
        live.error = None;
        if lens.representation.is_some() {
            lens.stage = LensStage::Completed;
        } else {
            lens.stage = LensStage::Ready;
        }
    })? {
        app.state::<AppState>().live_control.pause(operation_id)?;
        return Err("Lens operation was superseded before monitoring resumed".into());
    }
    apply_observation_coverage(app, operation_id, &coverage)?;
    request_immediate_refresh(app, operation_id)?;
    app.state::<AppState>().lens()
}

pub fn stop(app: &AppHandle, operation_id: Uuid) -> Result<(), String> {
    crate::session_controls::close_active(app);
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    state.live_control.stop(operation_id)?;
    Ok(())
}

fn require_live_operation(
    app: &AppHandle,
    operation_id: Uuid,
    lifecycle: LensMonitoringLifecycle,
) -> Result<LensState, String> {
    let lens = app.state::<AppState>().lens()?;
    if lens.operation_id != Some(operation_id) {
        return Err("Lens operation was superseded".into());
    }
    if lens.live.as_ref().map(|live| live.lifecycle) != Some(lifecycle) {
        return Err(format!(
            "Lens monitoring must be {lifecycle:?} for this operation"
        ));
    }
    Ok(lens)
}

fn build_observation(
    app: AppHandle,
    operation_id: Uuid,
    context_id: Uuid,
    observer_epoch: NonZeroU64,
    target_set: &LensTargetSet,
    start: ObservationStart,
) -> ObservationSetup {
    let (signal, receiver) = mpsc::channel(1);
    let mut registrations = Vec::with_capacity(target_set.targets.len());
    let mut source_receivers = Vec::with_capacity(target_set.targets.len());
    let mut authority = BTreeMap::new();
    let mut has_registration_diagnostics = false;
    let mut failures = Vec::new();

    for target in &target_set.targets {
        // Window IDs are unique inside LensTargetSet. Adding one avoids the nil UUID while keeping
        // the registration identity deterministic across Pause/Resume epochs.
        let source_registration_id = Uuid::from_u128(u128::from(target.identity.window_id) + 1);
        match platform::start_window_observation(
            operation_id,
            context_id,
            source_registration_id,
            observer_epoch,
            &target.identity,
        ) {
            Ok((registration, receiver, start)) => {
                has_registration_diagnostics |= !start.diagnostics.is_empty();
                for diagnostic in start.diagnostics {
                    eprintln!("Lens observer diagnostic for {}: {diagnostic}", target.id);
                }
                authority.insert(source_registration_id, target.identity.window_id);
                registrations.push(registration);
                source_receivers.push(receiver);
            }
            Err(error) => failures.push(format!("{}: {error}", target.id)),
        }
    }

    let coverage = ObservationCoverage {
        expected_sources: target_set.targets.len(),
        observing_sources: registrations.len(),
        has_registration_diagnostics,
        failures,
    };
    let scheduler = tauri::async_runtime::spawn(run_scheduler(
        app,
        ObservationSchedulerContext {
            operation_id,
            context_id,
            observer_epoch,
            source_window_authority: authority,
            coverage: coverage.clone(),
            start,
        },
        receiver,
    ));
    let forwarders = source_receivers
        .into_iter()
        .map(|receiver| spawn_source_forwarder(signal.clone(), receiver))
        .collect();
    let reconciler = spawn_periodic_reconciler(signal.clone());
    ObservationSetup {
        active: Some(ActiveObservation {
            signal,
            scheduler,
            reconciler,
            forwarders,
            registrations,
        }),
        coverage,
    }
}

fn spawn_periodic_reconciler(signal: mpsc::Sender<LiveSignal>) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let start = tokio::time::Instant::now() + PERIODIC_RECONCILIATION_INTERVAL;
        let mut interval = tokio::time::interval_at(start, PERIODIC_RECONCILIATION_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match signal.try_send(LiveSignal::PeriodicReconciliation) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    })
}

fn spawn_source_forwarder(
    signal: mpsc::Sender<LiveSignal>,
    mut receiver: WindowObservationReceiver,
) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = std::future::poll_fn(|context| receiver.poll_next(context)).await {
            match signal.try_send(LiveSignal::Invalidation(event)) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    })
}

async fn run_scheduler(
    app: AppHandle,
    context: ObservationSchedulerContext,
    mut receiver: mpsc::Receiver<LiveSignal>,
) {
    let ObservationSchedulerContext {
        operation_id,
        context_id,
        observer_epoch,
        source_window_authority,
        coverage,
        start,
    } = context;
    let mut scheduling = RefreshSchedulingState::new(start);
    while let Some(signal) = receiver.recv().await {
        let immediate = matches!(
            signal,
            LiveSignal::ImmediateRefresh | LiveSignal::PeriodicReconciliation
        );
        if !immediate
            && !valid_signal(
                &signal,
                operation_id,
                context_id,
                observer_epoch,
                &source_window_authority,
            )
        {
            continue;
        }
        if !immediate {
            let delay = tokio::time::sleep(OBSERVATION_COALESCING_INTERVAL);
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    _ = &mut delay => break,
                    next = receiver.recv() => match next {
                        Some(
                            LiveSignal::ImmediateRefresh
                            | LiveSignal::PeriodicReconciliation,
                        ) => break,
                        Some(next) if valid_signal(
                            &next,
                            operation_id,
                            context_id,
                            observer_epoch,
                            &source_window_authority,
                        ) => {}
                        Some(_) => {}
                        None => return,
                    }
                }
            }
        }

        let expected_revision = match app.state::<AppState>().lens() {
            Ok(lens)
                if lens.operation_id == Some(operation_id)
                    && lens.live.as_ref().is_some_and(|live| {
                        live.lifecycle == LensMonitoringLifecycle::Watching
                    }) =>
            {
                lens.context.as_ref().and_then(|context| {
                    (context.context_id == context_id).then_some(context.revision)
                })
            }
            _ => None,
        };
        let Some(expected_revision) = expected_revision else {
            continue;
        };

        match refresh_lens_context(app.clone(), operation_id, expected_revision).await {
            Ok(outcome) => {
                let _ = apply_observation_coverage(&app, operation_id, &coverage);
                let directive = scheduling.completed_refresh(outcome);
                match directive {
                    AgentSubmissionDirective::None => {}
                    AgentSubmissionDirective::LiveProjectionUpdate => {
                        let transform_app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                agent::transform_live_projection(transform_app, operation_id).await
                            {
                                eprintln!("Unable to update the live Lens representation: {error}");
                            }
                        });
                    }
                    AgentSubmissionDirective::RecoveryCheckpoint => {
                        let transform_app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                agent::transform_recovery_projection(transform_app, operation_id)
                                    .await
                            {
                                eprintln!(
                                    "Unable to recover the live Lens representation: {error}"
                                );
                            }
                        });
                    }
                }
            }
            Err(error) => {
                // Superseded and paused work is rejected by revision/lifecycle authority. Actual
                // extraction failures have already been published as explicit unverified state.
                eprintln!("Lens live refresh was not applied: {error}");
            }
        }
    }
}

fn valid_signal(
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

fn apply_observation_coverage(
    app: &AppHandle,
    operation_id: Uuid,
    coverage: &ObservationCoverage,
) -> Result<(), String> {
    let message = coverage.message();
    update_lens_state(app, operation_id, |lens| {
        let Some(live) = lens.live.as_mut() else {
            return;
        };
        if coverage.has_missing_sources() {
            live.health = LensSourceHealth::Unavailable;
            live.freshness = LensFreshness::Unverified;
            live.error = message;
        } else if coverage.has_registration_diagnostics && live.health == LensSourceHealth::Healthy
        {
            live.health = LensSourceHealth::Degraded;
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
