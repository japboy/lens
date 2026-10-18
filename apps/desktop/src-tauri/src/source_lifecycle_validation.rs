//! Explicit native lifecycle validation composition, separate from initial-only A11Y and real ACP.
use std::ffi::{OsStr, OsString};

pub(crate) const FLAG: &str = "LENS_VALIDATE_SOURCE_LIFECYCLE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Standard,
    SourceLifecycle,
}

impl Mode {
    pub(crate) fn parse(debug: bool, environment: &[(OsString, OsString)]) -> Result<Self, String> {
        let Some((_, value)) = environment.iter().find(|(key, _)| key == FLAG) else {
            return Ok(Self::Standard);
        };
        if !debug || value != OsStr::new("1") {
            return Err(format!("{FLAG}=1 requires a debug build"));
        }
        if environment.iter().any(|(key, _)| {
            key != FLAG
                && (key.to_string_lossy().starts_with("LENS_VALIDATE_")
                    || key == "LENS_DEBUG_SETTINGS")
        }) {
            return Err(format!(
                "{FLAG} cannot be combined with other validation flags or explicit targets"
            ));
        }
        Ok(Self::SourceLifecycle)
    }
    pub(crate) fn enabled(self) -> bool {
        self == Self::SourceLifecycle
    }
}

pub(crate) fn agent_services<R: tauri::Runtime>(mode: Mode) -> crate::agent::AgentServices<R> {
    #[cfg(debug_assertions)]
    if mode == Mode::SourceLifecycle {
        return crate::agent::AgentServices(std::sync::Arc::new(SourceOnlyAgentHost));
    }
    if mode != Mode::Standard {
        unreachable!("release composition rejects native validation before setup");
    }
    crate::agent::AgentServices(std::sync::Arc::new(crate::agent::DefaultAgentHost))
}

#[cfg(debug_assertions)]
pub(crate) struct SourceOnlyAgentHost;

#[cfg(debug_assertions)]
impl<R: tauri::Runtime> crate::agent::AgentHost<R> for SourceOnlyAgentHost {
    fn execution_policy(&self) -> crate::agent::AgentExecutionPolicy {
        crate::agent::AgentExecutionPolicy::SourceLifecycleValidation
    }
    fn resolve<'a>(
        &'a self,
        _: &'a tauri::AppHandle<R>,
        _: crate::model::AgentKind,
    ) -> crate::agent::HostFuture<'a, crate::agent_runtime::ResolvedAgentRuntime> {
        Box::pin(async { Err("Native source validation cannot acquire or launch an Agent".into()) })
    }
    fn resolve_installed<'a>(
        &'a self,
        _: &'a tauri::AppHandle<R>,
        _: crate::model::AgentKind,
    ) -> crate::agent::HostFuture<'a, Option<crate::agent_runtime::ResolvedAgentRuntime>> {
        Box::pin(async {
            Err("Native source validation cannot access an Agent installation".into())
        })
    }
    fn connect(
        &self,
        _: &crate::agent::AgentDescriptor,
        _: std::path::PathBuf,
        _: crate::agent_environment::EnvironmentPurpose,
    ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
        // An already-closed in-memory endpoint cannot start a process, authenticate or send inference.
        let (closed, peer) = agent_client_protocol::Channel::duplex();
        drop(peer);
        agent_client_protocol::DynConnectTo::new(closed)
    }
}

#[cfg(debug_assertions)]
mod stop {
    use crate::{agent, app_state::AppState, model::LensStage};
    use serde::Serialize;
    use tauri::{AppHandle, Manager};
    use usecase::read_admission::{CleanupOutcome, CleanupState, OperationAdmission};
    use uuid::Uuid;

    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StopPhase {
        Requested,
        AdmissionClosed,
        ObserversClosed,
        NativeReleaseReturned,
        CleanupRecorded,
        StateCleared,
        Returned,
    }
    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StopResult {
        Observed,
        Ok,
        Error,
        Incomplete,
    }
    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum OperationStatus {
        Missing,
        Foreign,
        Active,
        Terminal,
        Unavailable,
    }
    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum CleanupStatus {
        NotStarted,
        Running,
        ConfirmedClosed,
        UnconfirmedTermination,
        Unavailable,
    }
    #[derive(Debug, Clone, Copy, Serialize)]
    struct Metadata {
        admission: OperationStatus,
        cleanup: CleanupStatus,
        source_workers: Option<usize>,
        lens_operation_matches: Option<bool>,
        lens_stage: Option<LensStage>,
    }
    impl Default for Metadata {
        fn default() -> Self {
            Self {
                admission: OperationStatus::Unavailable,
                cleanup: CleanupStatus::Unavailable,
                source_workers: None,
                lens_operation_matches: None,
                lens_stage: None,
            }
        }
    }
    fn metadata<R: tauri::Runtime>(app: &AppHandle<R>, operation_id: Uuid) -> Metadata {
        let Some(state) = app.try_state::<AppState>() else {
            return Metadata::default();
        };
        let mut result = Metadata {
            source_workers: Some(
                state
                    .source_workers
                    .load(std::sync::atomic::Ordering::Acquire),
            ),
            ..Metadata::default()
        };
        // Diagnostics never wait for application locks or influence the operation.
        if let Ok(admission) = state.read_admission.try_lock() {
            match admission.as_ref() {
                None => result.admission = OperationStatus::Missing,
                Some(active) if active.operation_id() != operation_id => {
                    result.admission = OperationStatus::Foreign
                }
                Some(active) => {
                    result.admission = match active.operation_state() {
                        OperationAdmission::Active => OperationStatus::Active,
                        OperationAdmission::Terminal(_) => OperationStatus::Terminal,
                    };
                    result.cleanup = match active.cleanup_state() {
                        CleanupState::NotStarted => CleanupStatus::NotStarted,
                        CleanupState::Running => CleanupStatus::Running,
                        CleanupState::Finished(CleanupOutcome::ConfirmedClosed) => {
                            CleanupStatus::ConfirmedClosed
                        }
                        CleanupState::Finished(CleanupOutcome::UnconfirmedTermination) => {
                            CleanupStatus::UnconfirmedTermination
                        }
                    };
                }
            }
        }
        if let Ok(runtime) = state.runtime.try_read() {
            result.lens_operation_matches = Some(runtime.lens.operation_id == Some(operation_id));
            result.lens_stage = Some(runtime.lens.stage);
        }
        result
    }
    #[derive(Debug, Clone, Copy, Serialize)]
    struct Checkpoint {
        operation_id: Uuid,
        attempt_id: Uuid,
        phase: StopPhase,
        result: StopResult,
        metadata: Metadata,
    }
    fn emit(checkpoint: &Checkpoint) {
        use std::io::Write;
        if let Ok(json) = serde_json::to_string(checkpoint) {
            let _ = writeln!(std::io::stdout(), "LENS_SOURCE_LIFECYCLE_STOP={json}");
        }
    }
    pub(crate) struct StopProbe {
        enabled: bool,
        last: Checkpoint,
        finished: bool,
        writer: fn(&Checkpoint),
    }
    impl StopProbe {
        pub(crate) fn new<R: tauri::Runtime>(app: &AppHandle<R>, operation_id: Uuid) -> Self {
            let enabled = app
                .try_state::<agent::AgentServices<R>>()
                .is_some_and(|host| {
                    host.0.execution_policy()
                        == agent::AgentExecutionPolicy::SourceLifecycleValidation
                });
            let mut probe = Self {
                enabled,
                last: Checkpoint {
                    operation_id,
                    attempt_id: Uuid::new_v4(),
                    phase: StopPhase::Requested,
                    result: StopResult::Observed,
                    metadata: Metadata::default(),
                },
                finished: false,
                writer: emit,
            };
            probe.record(app, StopPhase::Requested, StopResult::Observed);
            probe
        }
        pub(crate) fn record<R: tauri::Runtime>(
            &mut self,
            app: &AppHandle<R>,
            phase: StopPhase,
            result: StopResult,
        ) {
            if self.enabled {
                self.last = Checkpoint {
                    operation_id: self.last.operation_id,
                    attempt_id: self.last.attempt_id,
                    phase,
                    result,
                    metadata: metadata(app, self.last.operation_id),
                };
                (self.writer)(&self.last);
            }
        }
        pub(crate) fn finish<R: tauri::Runtime>(&mut self, app: &AppHandle<R>, success: bool) {
            self.record(
                app,
                StopPhase::Returned,
                if success {
                    StopResult::Ok
                } else {
                    StopResult::Error
                },
            );
            self.finished = true;
        }
    }
    impl Drop for StopProbe {
        fn drop(&mut self) {
            if self.enabled && !self.finished {
                // Stored metadata only: no app locks, native calls, cleanup or status promotion.
                (self.writer)(&Checkpoint {
                    result: StopResult::Incomplete,
                    ..self.last
                });
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::RefCell;
        thread_local! { static EVENTS: RefCell<Vec<Checkpoint>> = const { RefCell::new(Vec::new()) }; }
        fn collect(event: &Checkpoint) {
            EVENTS.with(|events| events.borrow_mut().push(*event));
        }
        #[test]
        fn incomplete_drop_uses_last_metadata_without_promoting_cleanup() {
            EVENTS.with(|events| events.borrow_mut().clear());
            let mut probe = StopProbe {
                enabled: true,
                last: Checkpoint {
                    operation_id: Uuid::from_u128(1),
                    attempt_id: Uuid::from_u128(2),
                    phase: StopPhase::CleanupRecorded,
                    result: StopResult::Ok,
                    metadata: Metadata {
                        cleanup: CleanupStatus::UnconfirmedTermination,
                        source_workers: Some(1),
                        ..Metadata::default()
                    },
                },
                finished: false,
                writer: collect,
            };
            (probe.writer)(&probe.last);
            probe.last.metadata.source_workers = Some(0);
            drop(probe);
            EVENTS.with(|events| {
                let events = events.borrow();
                assert_eq!(events.len(), 2);
                assert_eq!(events[1].result, StopResult::Incomplete);
                assert_eq!(
                    events[1].metadata.cleanup,
                    CleanupStatus::UnconfirmedTermination
                );
                assert_eq!(events[1].phase, StopPhase::CleanupRecorded);
            });
        }
        #[test]
        fn completed_or_disabled_probe_does_not_emit_incomplete() {
            EVENTS.with(|events| events.borrow_mut().clear());
            for (enabled, finished) in [(true, true), (false, false)] {
                drop(StopProbe {
                    enabled,
                    finished,
                    last: Checkpoint {
                        operation_id: Uuid::from_u128(1),
                        attempt_id: Uuid::from_u128(2),
                        phase: StopPhase::Returned,
                        result: StopResult::Ok,
                        metadata: Metadata::default(),
                    },
                    writer: collect,
                });
            }
            EVENTS.with(|events| assert!(events.borrow().is_empty()));
        }
        #[test]
        fn real_host_metadata_is_bounded_and_failed_cleanup_is_not_success() {
            EVENTS.with(|events| events.borrow_mut().clear());
            let operation_id = Uuid::from_u128(1);
            let state = crate::test_support::state();
            state.begin_source_operation(operation_id).unwrap();
            let app = tauri::test::mock_builder()
                .manage(state)
                .manage(super::super::agent_services::<tauri::test::MockRuntime>(
                    super::super::Mode::SourceLifecycle,
                ))
                .build(crate::product_context())
                .unwrap();
            let mut probe = StopProbe::new(app.handle(), operation_id);
            probe.writer = collect;
            let state = app.state::<AppState>();
            let admission = state.read_admission.lock().unwrap();
            probe.record(
                app.handle(),
                StopPhase::AdmissionClosed,
                StopResult::Observed,
            );
            assert_eq!(probe.last.metadata.admission, OperationStatus::Unavailable);
            drop(admission);
            state
                .terminate_source_operation(
                    operation_id,
                    usecase::read_admission::OperationTerminalReason::Cancelled,
                )
                .unwrap();
            state.begin_source_cleanup(operation_id).unwrap();
            state.finish_source_cleanup(operation_id, false).unwrap();
            probe.record(
                app.handle(),
                StopPhase::CleanupRecorded,
                StopResult::Observed,
            );
            assert_eq!(probe.last.metadata.admission, OperationStatus::Terminal);
            assert_eq!(
                probe.last.metadata.cleanup,
                CleanupStatus::UnconfirmedTermination
            );
            probe.finish(app.handle(), false);
            drop(probe);
            EVENTS.with(|events| {
                let events = events.borrow();
                assert_eq!(events.last().unwrap().phase, StopPhase::Returned);
                assert_eq!(events.last().unwrap().result, StopResult::Error);
                let json = serde_json::to_value(events.last().unwrap()).unwrap();
                assert_eq!(json["metadata"]["cleanup"], "unconfirmed_termination");
                assert!(json.get("receipt").is_none());
                assert!(json.get("window_id").is_none());
            });
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) use stop::{StopPhase, StopProbe, StopResult};

#[cfg(test)]
mod tests {
    use super::*;
    fn environment(entries: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        entries
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }
    #[test]
    fn dedicated_mode_is_debug_only_exact_and_exclusive() {
        assert_eq!(Mode::parse(true, &[]).unwrap(), Mode::Standard);
        assert_eq!(
            Mode::parse(true, &environment(&[(FLAG, "1")])).unwrap(),
            Mode::SourceLifecycle
        );
        assert!(Mode::parse(false, &environment(&[(FLAG, "1")])).is_err());
        for value in ["", "0", "true", " 1"] {
            assert!(Mode::parse(true, &environment(&[(FLAG, value)])).is_err());
        }
        for key in [
            "LENS_VALIDATE_A11Y",
            "LENS_VALIDATE_ACP",
            "LENS_VALIDATE_TARGET",
            "LENS_VALIDATE_MCP_APPS",
            "LENS_VALIDATE_RUNTIME",
            "LENS_DEBUG_SETTINGS",
        ] {
            assert!(Mode::parse(true, &environment(&[(FLAG, "1"), (key, "1")])).is_err());
            assert_eq!(
                Mode::parse(true, &environment(&[(key, "1")])).unwrap(),
                Mode::Standard
            );
        }
    }
    #[test]
    fn only_explicit_source_policy_permits_selection_without_authentication() {
        use crate::agent::AgentExecutionPolicy;
        assert!(!AgentExecutionPolicy::Product.allows_target_selection(false));
        assert!(AgentExecutionPolicy::Product.allows_target_selection(true));
        #[cfg(debug_assertions)]
        assert!(AgentExecutionPolicy::SourceLifecycleValidation.allows_target_selection(false));
        assert_eq!(
            crate::agent::AgentHost::<tauri::test::MockRuntime>::execution_policy(
                &crate::agent::DefaultAgentHost
            ),
            AgentExecutionPolicy::Product
        );
    }
}
