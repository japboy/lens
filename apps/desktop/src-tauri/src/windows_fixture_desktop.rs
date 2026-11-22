//! Development-only Desktop composition for the cooperative Win32 fixture.

use crate::initial_publication;
use adapter_platform_windows::{
    marker_oracle::{self, Phase, Rect},
    selection_cleanup::SelectionPhase,
    Fixture, FixturePlatform, FixtureReadProbe,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use port_platform::{
    authority::{ReadSequence, TargetReadKey},
    observation::{Observation, ObservationRequest, ObservationSession},
    selection::{TargetSelection, WindowPickerReply},
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    future::{poll_fn, Future},
    num::NonZeroU64,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;
use usecase::{
    context::{build_context, BlockingExecutor, ContextBuildRequest, WindowAccessMode},
    lens::{LensInput, LensMediaGeometry, LensMediaPayload, LensTargetSet, TargetNamespace},
    live_sync::LensAgentProjection,
    model::{LensStage, LensState},
    platform::picker_reply,
    read_admission::{
        CleanupOutcome, CleanupState, OperationAdmission, OperationTerminalReason, ReadAdmission,
    },
};
use uuid::Uuid;

const WINDOW: &str = "fixture";
const ACQUISITION_DEADLINE: Duration = Duration::from_secs(2);
const WITHDRAWAL_RECEIPT_LIMIT: Duration = Duration::from_secs(2);

struct DesktopExecutor;

impl BlockingExecutor for DesktopExecutor {
    fn run<T: Send + 'static>(
        &self,
        _: Option<TargetReadKey>,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send>> {
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(work)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

struct ContextCandidate {
    state: LensState,
    reads: Vec<TargetReadKey>,
    payloads: Vec<LensMediaPayload>,
    digest: String,
}

const MARKER: &str = "Lens geometry marker";
const UPDATED_MARKER: &str = "Lens geometry marker updated";

fn marker(lens: &LensState) -> Option<&str> {
    lens.context
        .as_ref()?
        .sources
        .first()?
        .capture
        .nodes
        .iter()
        .filter_map(|node| node.title.as_deref())
        .find(|title| matches!(*title, MARKER | UPDATED_MARKER))
}

struct Advance {
    read: TargetReadKey,
    rendered: bool,
    next: oneshot::Sender<()>,
}

#[derive(Default)]
struct RuntimeState {
    selection_phase: SelectionPhase,
    lens: LensState,
    admission: Option<ReadAdmission>,
    payloads: BTreeMap<String, LensMediaPayload>,
    held: Option<ContextCandidate>,
    active_read: Option<ActiveRead>,
    published: Option<TargetReadKey>,
    advance: Option<Advance>,
    withdrawal_receipt: Option<oneshot::Sender<()>>,
}

impl RuntimeState {
    fn commit(
        &mut self,
        platform: &FixturePlatform,
        candidate: &ContextCandidate,
    ) -> Result<bool, String> {
        let Some(operation) = candidate.state.operation_id else {
            return Ok(false);
        };
        let [read] = candidate.reads.as_slice() else {
            return Ok(false);
        };
        let read = *read;
        let follows_current = if read.sequence == ReadSequence::FIRST {
            initial_publication::is_pending(&self.lens, operation)
        } else {
            self.lens.operation_id == Some(operation)
                && self.lens.stage == LensStage::Ready
                && self.published.is_some_and(|previous| {
                    previous.target == read.target
                        && previous.sequence.checked_next() == Some(read.sequence)
                })
        };
        if !follows_current
            || candidate.state.context.as_ref().map(|ctx| ctx.revision) != Some(read.sequence.get())
        {
            return Ok(false);
        }
        let Some(admission) = self.admission.as_mut() else {
            return Ok(false);
        };
        initial_publication::validate_result(operation, &candidate.state)?;
        if !initial_publication::allows_reads(admission, &candidate.state, &candidate.reads, true) {
            return Ok(false);
        }
        let payloads = initial_publication::payloads(candidate.payloads.clone())?;
        if payloads.len() != 1 {
            return Err("the fixture requires a paired image before publication".into());
        }
        platform
            .approve_read_publication(read)
            .map_err(|error| error.to_string())?;
        if !admission.accept_all(&candidate.reads) {
            return Ok(false);
        }
        self.payloads = payloads;
        self.lens = candidate.state.clone();
        self.published = Some(read);
        Ok(true)
    }

    fn withdraw(&mut self, stage: LensStage, error: String) -> Option<TargetReadKey> {
        self.lens = LensState {
            operation_id: self.lens.operation_id,
            stage,
            error: Some(error),
            ..LensState::default()
        };
        self.advance = None;
        self.payloads.clear();
        self.published.take()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            operation: self.lens.operation_id,
            stage: self.lens.stage,
            read: self.published,
            marker: marker(&self.lens).map(str::to_owned),
            title: self
                .lens
                .context
                .as_ref()
                .and_then(|context| context.sources.first())
                .map(|source| source.source.window_title.clone()),
            png: self
                .payloads
                .values()
                .next()
                .map(|payload| payload.data.clone()),
            error: self.lens.error.clone(),
        }
    }
}

#[derive(Clone, Serialize)]
struct Snapshot {
    operation: Option<Uuid>,
    stage: LensStage,
    read: Option<TargetReadKey>,
    marker: Option<String>,
    title: Option<String>,
    png: Option<String>,
    error: Option<String>,
}

struct ActiveRead {
    key: TargetReadKey,
    platform: Arc<FixturePlatform>,
    action: Action,
    started: Instant,
}

struct Host {
    bins: PathBuf,
    output: PathBuf,
    runtime: Mutex<RuntimeState>,
    selection: tokio::sync::Mutex<()>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    SelectA,
    RejectA,
    SelectB,
    StopA,
    DeadlineA,
    LateA,
    ContinuousA,
    WorkerExitA,
    StallRefreshA,
}

impl Action {
    fn read_probe(self) -> FixtureReadProbe {
        match self {
            Self::StopA | Self::DeadlineA => FixtureReadProbe::Stall,
            Self::LateA => FixtureReadProbe::Late,
            _ => FixtureReadProbe::Normal,
        }
    }

    fn can_stop(self) -> bool {
        matches!(self, Self::StopA | Self::LateA | Self::ContinuousA)
    }
}

fn log(event: &str, detail: serde_json::Value) {
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    println!(
        "{}",
        json!({"event":event,"elapsed_ms":elapsed,"unix_ms":unix_ms,"detail":detail})
    );
}

impl Host {
    fn runtime(&self) -> Result<std::sync::MutexGuard<'_, RuntimeState>, String> {
        self.runtime
            .lock()
            .map_err(|_| "fixture Desktop runtime poisoned".into())
    }

    fn waiting_to_stop(&self) -> Result<Option<Uuid>, String> {
        let runtime = self.runtime()?;
        let Some(read) = runtime.active_read.as_ref() else {
            return Ok(None);
        };
        Ok((read.action.can_stop()
            && read.platform.validation_waiting()
            && runtime
                .admission
                .as_ref()
                .is_some_and(|admission| admission.admits(read.key)))
        .then_some(read.key.target.operation_id))
    }

    fn finish_read(
        &self,
        operation: Uuid,
        reason: OperationTerminalReason,
    ) -> Result<bool, String> {
        let mut runtime = self.runtime()?;
        let Some(read) = runtime
            .active_read
            .as_ref()
            .filter(|read| read.key.target.operation_id == operation)
        else {
            return Ok(false);
        };
        let waiting = read.platform.validation_waiting();
        if reason == OperationTerminalReason::Cancelled && (!read.action.can_stop() || !waiting) {
            return Ok(false);
        }
        let (key, platform, action, elapsed) = (
            read.key,
            read.platform.clone(),
            read.action,
            read.started.elapsed(),
        );
        let Some(admission) = runtime
            .admission
            .as_mut()
            .filter(|admission| admission.admits(key))
        else {
            return Ok(false);
        };
        admission.terminate(reason);
        let withdrawn = if reason == OperationTerminalReason::Cancelled {
            runtime.withdraw(LensStage::Cancelled, "Acquisition stopped.".into())
        } else {
            runtime.withdraw(LensStage::Failed, "Acquisition deadline elapsed.".into())
        };
        log(
            "desktop-read-terminated",
            json!({
                "read":key, "reason":format!("{reason:?}"), "helper_waiting":waiting, "elapsed_ms":elapsed.as_millis(), "withdrawn":withdrawn
            }),
        );
        // Deliberately let one real native result through the transport only after admission closes.
        match action {
            Action::LateA => platform.deliver_validation_result(),
            // The later read remains held until WebView acknowledges the withdrawal.
            Action::ContinuousA => {}
            _ => platform.interrupt_validation_read(),
        }
        Ok(true)
    }

    async fn select(&self, window: &WebviewWindow, action: Action) -> Result<Snapshot, String> {
        let _selection = self
            .selection
            .try_lock()
            .map_err(|_| "fixture selection is already active")?;
        if self.runtime()?.admission.as_ref().is_some_and(|admission| {
            matches!(
                admission.cleanup_state(),
                CleanupState::Running
                    | CleanupState::Finished(CleanupOutcome::UnconfirmedTermination)
            )
        }) {
            log(
                "desktop-reselection-denied",
                json!({"reason":"prior helper termination is unconfirmed"}),
            );
            return Err(
                "Previous helper termination could not be confirmed. Selection is unavailable."
                    .into(),
            );
        }
        let operation = Uuid::new_v4();
        let fixture = if matches!(action, Action::SelectB) {
            Fixture::B
        } else {
            Fixture::A
        };
        let platform =
            FixturePlatform::new(&self.bins, fixture).map_err(|error| error.to_string())?;
        let second = ReadSequence::FIRST
            .checked_next()
            .expect("bounded second read");
        let platform = Arc::new(match action {
            Action::ContinuousA => platform.with_continuous_reads().with_read_probe_at(
                second.checked_next().expect("bounded third read"),
                FixtureReadProbe::Late,
            ),
            Action::StallRefreshA => platform
                .with_continuous_reads()
                .with_read_probe_at(second, FixtureReadProbe::Stall),
            Action::WorkerExitA => platform.with_continuous_reads(),
            _ => platform.with_read_probe(action.read_probe()),
        });
        {
            let mut runtime = self.runtime()?;
            runtime.lens = LensState {
                operation_id: Some(operation),
                stage: LensStage::Extracting,
                ..LensState::default()
            };
            runtime.payloads.clear();
            runtime.active_read = None;
            runtime.published = None;
            runtime.advance = None;
            runtime.withdrawal_receipt = None;
            runtime.admission = Some(ReadAdmission::new(operation));
            runtime.selection_phase = SelectionPhase::Pending;
        }
        log(
            "desktop-selection-started",
            json!({"operation":operation, "fixture":format!("{fixture:?}"),"action":format!("{action:?}")}),
        );
        let result = self.acquire(&platform, operation, action).await;
        {
            let mut runtime = self.runtime()?;
            runtime.active_read = None;
            runtime.advance = None;
            runtime.withdrawal_receipt = None;
            let mut already_terminal = false;
            if let Some(admission) = runtime.admission.as_mut() {
                already_terminal =
                    matches!(admission.operation_state(), OperationAdmission::Terminal(_));
                let reason = match result.as_ref() {
                    Ok(reason) => *reason,
                    Err(_) => OperationTerminalReason::AuthorityUncertain,
                };
                admission.terminate(reason);
                admission.begin_cleanup();
            }
            if let Err(error) = result {
                log(
                    "desktop-acquisition-ended",
                    json!({"operation":operation,"detail":error,"terminal_already":already_terminal}),
                );
                if !already_terminal || matches!(action, Action::ContinuousA) {
                    runtime.withdraw(LensStage::Failed, error);
                }
            }
        }
        log("desktop-cleanup-start", json!({"operation":operation}));
        let cleanup =
            tauri::async_runtime::spawn_blocking(move || platform.release_operation(operation))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
        let snapshot = {
            let mut runtime = self.runtime()?;
            let selection_phase = runtime.selection_phase;
            if let Some(admission) = runtime.admission.as_mut() {
                admission.finish_cleanup(if selection_phase.confirms_release(cleanup.is_ok()) {
                    CleanupOutcome::ConfirmedClosed
                } else {
                    CleanupOutcome::UnconfirmedTermination
                });
                log(
                    "desktop-cleanup",
                    json!({"operation":operation,"selection_phase":format!("{selection_phase:?}"),"outcome":format!("{:?}",admission.cleanup_state()),"error":cleanup.as_ref().err().map(ToString::to_string)}),
                );
            }
            runtime.snapshot()
        };
        cleanup?;
        window
            .emit("fixture-snapshot", &snapshot)
            .map_err(|error| error.to_string())?;
        Ok(snapshot)
    }

    async fn acquire(
        &self,
        platform: &Arc<FixturePlatform>,
        operation: Uuid,
        action: Action,
    ) -> Result<OperationTerminalReason, String> {
        let picker = platform.clone();
        let reply = tauri::async_runtime::spawn_blocking(move || {
            tauri::async_runtime::block_on(picker.pick(operation))
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        let target = match &reply {
            WindowPickerReply::Cancelled => {
                let mut runtime = self.runtime()?;
                runtime.selection_phase = SelectionPhase::Cancelled;
                runtime.lens.stage = LensStage::Cancelled;
                log("desktop-cancelled", json!({"operation":operation}));
                return Ok(OperationTerminalReason::Cancelled);
            }
            WindowPickerReply::Selected { windows } if windows.len() == 1 => {
                self.runtime()?.selection_phase = SelectionPhase::Registered;
                windows[0].target
            }
            _ => return Err("fixture picker did not return one registered target".into()),
        };
        let set = LensTargetSet::try_new_for_namespace(
            operation,
            picker_reply(reply)
                .into_selected()?
                .ok_or("fixture selection absent")?,
            TargetNamespace::Windows,
        )
        .map_err(|error| error.to_string())?;
        let id = set.targets[0].id.clone();
        {
            let mut runtime = self.runtime()?;
            let admission = runtime
                .admission
                .as_mut()
                .ok_or("Desktop admission missing")?;
            admission.register(id.clone(), target)?;
        }
        let observation_id = Uuid::new_v4();
        let mut observation = platform
            .observe(ObservationRequest {
                target,
                context_id: operation,
                source_registration_id: observation_id,
                observer_epoch: NonZeroU64::MIN,
            })
            .map_err(|error| error.to_string())?;
        log(
            "desktop-registered",
            json!({"target":target,"observation":observation_id}),
        );
        if matches!(
            action,
            Action::ContinuousA | Action::WorkerExitA | Action::StallRefreshA
        ) {
            return self
                .continuous(platform, &set, &mut observation, action)
                .await;
        }
        let read = self.start_read(platform, &set, action, 1)?;
        let candidate = self.build_candidate(platform, &set, read, action).await?;
        if matches!(action, Action::LateA) {
            return self.reject_late(platform, candidate, read);
        }
        if matches!(action, Action::RejectA) {
            let owner = platform.clone();
            tauri::async_runtime::spawn_blocking(move || owner.terminate_owned_fixture(target))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            let native = tokio::time::timeout(
                Duration::from_secs(2),
                poll_fn(|cx| observation.events.poll_next(cx)),
            )
            .await
            .map_err(|error| error.to_string())?;
            let mut runtime = self.runtime()?;
            match runtime.commit(platform, &candidate) {
                Err(error) if error.contains("authority lost") => {
                    log(
                        "desktop-rejected-owner-loss",
                        json!({"read":read,"notification":native,"detail":error}),
                    );
                }
                _ => return Err("expected Desktop native gate to reject the held read".into()),
            }
            runtime.held = Some(candidate);
            runtime.withdraw(
                LensStage::Failed,
                "Source closed. Select a fixture again.".into(),
            );
            return Ok(OperationTerminalReason::AuthorityUncertain);
        }

        let mut runtime = self.runtime()?;
        if let Some(held) = runtime.held.take() {
            if runtime.commit(platform, &held)? {
                return Err("Desktop admitted obsolete context after reselection".into());
            }
            log(
                "desktop-rejected-old-generation",
                json!({"old":held.reads,"current":read}),
            );
        }
        if !runtime.commit(platform, &candidate)? {
            return Err("Desktop initial publication was rejected".into());
        }
        if runtime.commit(platform, &candidate)? {
            return Err("Desktop duplicated its initial publication".into());
        }
        self.export(&candidate, read, &operation.to_string(), None)?;
        Ok(OperationTerminalReason::Completed)
    }

    fn start_read(
        &self,
        platform: &Arc<FixturePlatform>,
        set: &LensTargetSet,
        action: Action,
        revision: u64,
    ) -> Result<TargetReadKey, String> {
        let mut runtime = self.runtime()?;
        let admission = runtime
            .admission
            .as_mut()
            .ok_or("Desktop admission missing")?;
        let key = admission.issue(&set.targets[0].id, revision)?;
        if !admission.start(key) {
            return Err("Desktop reading could not begin".into());
        }
        runtime.active_read = Some(ActiveRead {
            key,
            platform: platform.clone(),
            action,
            started: Instant::now(),
        });
        log(
            "desktop-read-started",
            json!({"read":key,"revision":revision}),
        );
        Ok(key)
    }

    async fn build_candidate(
        &self,
        platform: &Arc<FixturePlatform>,
        set: &LensTargetSet,
        read: TargetReadKey,
        action: Action,
    ) -> Result<ContextCandidate, String> {
        let id = set.targets[0].id.clone();
        let operation = read.target.operation_id;
        let revision = read.sequence.get();
        let mut acquisition = Box::pin(build_context(
            &DesktopExecutor,
            platform.clone(),
            platform.clone(),
            ContextBuildRequest {
                operation_id: operation,
                context_revision: revision,
                source_revisions: BTreeMap::from([(id.clone(), revision)]),
                target_set: set,
                access_mode: WindowAccessMode::Registered {
                    reads: BTreeMap::from([(id, read)]),
                },
            },
        ));
        let stalled_refresh = matches!(action, Action::StallRefreshA) && revision == 2;
        let built = if stalled_refresh || matches!(action, Action::DeadlineA) {
            tokio::select! {
                built = &mut acquisition => built,
                () = tokio::time::sleep(ACQUISITION_DEADLINE) => {
                    if !self.finish_read(operation, OperationTerminalReason::TimedOut)? {
                        return Err("deadline could not close the active read".into());
                    }
                    if stalled_refresh {
                        self.wait_for_withdrawal(read).await?;
                    }
                    acquisition.await
                }
            }
        } else {
            acquisition.await
        }?;
        let input =
            LensInput::from_context(&built.context).ok_or("fixture context has no usable input")?;
        let projection =
            LensAgentProjection::from_input(&input, &built.target_set, &built.payloads)
                .map_err(|error| error.to_string())?;
        if built.payloads.len() != 1 || built.context.media.len() != 1 {
            return Err("expected real UIA text and one captured image".into());
        }
        let candidate = ContextCandidate {
            state: LensState {
                operation_id: Some(operation),
                stage: LensStage::Ready,
                context: Some(built.context.into()),
                input: Some(input.into()),
                projection: Some(projection.projection_ref(NonZeroU64::MIN)),
                target_set: Some(built.target_set),
                ..LensState::default()
            },
            reads: built.reads,
            payloads: built.payloads,
            digest: projection.digest().as_str().to_owned(),
        };
        let expected = if revision.is_multiple_of(2) {
            UPDATED_MARKER
        } else {
            MARKER
        };
        if marker(&candidate.state) != Some(expected) || candidate.reads != [read] {
            return Err("fixture UIA marker or paired read did not match the revision".into());
        }
        log(
            "desktop-context-built",
            json!({"read":read, "digest":projection.digest().as_str()}),
        );
        Ok(candidate)
    }

    fn reject_late(
        &self,
        platform: &FixturePlatform,
        candidate: ContextCandidate,
        read: TargetReadKey,
    ) -> Result<OperationTerminalReason, String> {
        let mut runtime = self.runtime()?;
        if !runtime.admission.as_ref().is_some_and(|admission| {
            admission.operation_state()
                == OperationAdmission::Terminal(OperationTerminalReason::Cancelled)
                && !admission.allows_all(&candidate.reads)
        }) || runtime.commit(platform, &candidate)?
        {
            return Err("late native result did not meet the Stop rejection contract".into());
        }
        log(
            "desktop-rejected-late-result",
            json!({"read":read,"digest":candidate.digest}),
        );
        runtime.held = Some(candidate);
        Ok(OperationTerminalReason::Cancelled)
    }

    async fn continuous(
        &self,
        platform: &Arc<FixturePlatform>,
        set: &LensTargetSet,
        observation: &mut ObservationSession,
        action: Action,
    ) -> Result<OperationTerminalReason, String> {
        let publications = if matches!(action, Action::ContinuousA) {
            2
        } else {
            1
        };
        for revision in 1..=publications {
            let read = self.start_read(platform, set, action, revision)?;
            let candidate = self.build_candidate(platform, set, read, action).await?;
            let raw = platform
                .validation_frame_png(read)
                .map_err(|error| error.to_string())?;
            let capture = STANDARD
                .decode(&candidate.payloads[0].data)
                .map_err(|error| error.to_string())?;
            let context = candidate
                .state
                .context
                .as_ref()
                .ok_or("fixture context missing")?;
            let LensMediaGeometry::Acquired { crop, .. } = &context.media[0].geometry else {
                return Err("fixture media has no acquired crop geometry".into());
            };
            let region = Rect {
                x: crop.x,
                y: crop.y,
                width: crop.width,
                height: crop.height,
            };
            let phase = if revision == 1 {
                Phase::A
            } else {
                Phase::Updated
            };
            let found = marker_oracle::verify_pngs(&raw, &capture, region, phase)?;
            log(
                "desktop-oracle-verified",
                json!({"read":read,"crop":[found.x,found.y,found.width,found.height]}),
            );
            let (next, proceed) = oneshot::channel();
            {
                let mut runtime = self.runtime()?;
                if !runtime.commit(platform, &candidate)? {
                    return Err("Desktop continuous publication rejected".into());
                }
                let prefix = format!("{}-read-{revision}", read.target.operation_id);
                self.export(&candidate, read, &prefix, Some(&raw))?;
                runtime.active_read = None;
                runtime.advance = Some(Advance {
                    read,
                    rendered: false,
                    next,
                });
            }
            tokio::select! {
                ready = tokio::time::timeout(Duration::from_secs(25), proceed) => {
                    ready.map_err(|_| "Timed out waiting for the next fixture read.")?
                        .map_err(|_| "Fixture read continuation was closed.")?;
                }
                event = poll_fn(|cx| observation.events.poll_next(cx)) => {
                    log("desktop-continuous-invalidated", json!({"read":read,"notification":event}));
                    return Err("Source closed. Select a fixture again.".into());
                }
            }
        }
        match action {
            Action::WorkerExitA => self.worker_exit(platform, set, observation).await,
            Action::StallRefreshA => self.stalled_refresh(platform, set).await,
            _ => {
                let read = self.start_read(platform, set, action, 3)?;
                let candidate = self.build_candidate(platform, set, read, action).await?;
                self.reject_late(platform, candidate, read)
            }
        }
    }

    async fn wait_for_withdrawal(&self, affected: TargetReadKey) -> Result<(), String> {
        let (sender, receipt) = oneshot::channel();
        self.runtime()?.withdrawal_receipt = Some(sender);
        let observed = tokio::time::timeout(WITHDRAWAL_RECEIPT_LIMIT, receipt).await;
        self.runtime()?.withdrawal_receipt = None;
        if !matches!(observed, Ok(Ok(()))) {
            log("webview-withdrawal-unconfirmed", json!({"read":affected}));
            return Err("WebView did not confirm the withdrawal.".into());
        }
        Ok(())
    }

    async fn worker_exit(
        &self,
        platform: &Arc<FixturePlatform>,
        set: &LensTargetSet,
        observation: &mut ObservationSession,
    ) -> Result<OperationTerminalReason, String> {
        let read = self
            .runtime()?
            .published
            .ok_or("Fixture had no visible result.")?;
        log(
            "desktop-fault-triggered",
            json!({"kind":"worker_exit","read":read}),
        );
        let adapter = platform.clone();
        let worker =
            tauri::async_runtime::spawn_blocking(move || adapter.exit_owned_worker(read.target));
        let native = tokio::time::timeout(
            Duration::from_secs(2),
            poll_fn(|cx| observation.events.poll_next(cx)),
        )
        .await;
        if let Ok(ref notification) = native {
            log(
                "desktop-native-closure-observed",
                json!({"read":read,"notification":notification}),
            );
        }
        let reason = OperationTerminalReason::AuthorityUncertain;
        {
            let mut runtime = self.runtime()?;
            let admission = runtime
                .admission
                .as_mut()
                .ok_or("Desktop admission missing")?;
            admission.terminate(reason);
            let denied = admission.issue(&set.targets[0].id, 2).is_err();
            let withdrawn = runtime.withdraw(
                LensStage::Failed,
                "Acquisition helper closed. Select a fixture again.".into(),
            );
            log(
                "desktop-authority-closed",
                json!({"read":read,"withdrawn":withdrawn,"next_read_denied":denied}),
            );
            if !denied {
                return Err("Lost registration admitted a new read.".into());
            }
        }
        let rendered = self.wait_for_withdrawal(read).await;
        let pid = worker
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        native.map_err(|error| error.to_string())?;
        let rejected = platform
            .approve_read_publication(read)
            .err()
            .ok_or("Closed worker admitted the withdrawn read.")?;
        log(
            "desktop-rejected-closed-worker",
            json!({"read":read,"pid":pid,"detail":rejected.to_string()}),
        );
        rendered?;
        Ok(reason)
    }

    async fn stalled_refresh(
        &self,
        platform: &Arc<FixturePlatform>,
        set: &LensTargetSet,
    ) -> Result<OperationTerminalReason, String> {
        let action = Action::StallRefreshA;
        let read = self.start_read(platform, set, action, 2)?;
        log(
            "desktop-fault-triggered",
            json!({"kind":"stalled_refresh","read":read,"deadline_ms":ACQUISITION_DEADLINE.as_millis()}),
        );
        let error = match self.build_candidate(platform, set, read, action).await {
            Ok(_) => return Err("Stalled fixture unexpectedly produced a paired result.".into()),
            Err(error) => error,
        };
        let runtime = self.runtime()?;
        let reason = OperationTerminalReason::TimedOut;
        if !runtime.admission.as_ref().is_some_and(|admission| {
            admission.operation_state() == OperationAdmission::Terminal(reason)
                && !admission.admits(read)
        }) || !error.contains("validation read interrupted")
        {
            return Err(error);
        }
        log(
            "desktop-rejected-stalled-read",
            json!({"read":read,"detail":error}),
        );
        Ok(reason)
    }

    fn export(
        &self,
        candidate: &ContextCandidate,
        read: TargetReadKey,
        prefix: &str,
        raw: Option<&[u8]>,
    ) -> Result<(), String> {
        let context = candidate
            .state
            .context
            .as_ref()
            .ok_or("fixture context missing")?;
        let selection = candidate
            .state
            .target_set
            .as_ref()
            .ok_or("fixture selection missing")?;
        fs::write(
            self.output.join(format!("{prefix}-selection.json")),
            serde_json::to_vec_pretty(selection).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            self.output.join(format!("{prefix}-context.json")),
            serde_json::to_vec_pretty(context).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let capture = STANDARD
            .decode(&candidate.payloads[0].data)
            .map_err(|error| error.to_string())?;
        fs::write(self.output.join(format!("{prefix}-capture.png")), &capture)
            .map_err(|error| error.to_string())?;
        if let Some(raw) = raw {
            fs::write(self.output.join(format!("{prefix}-frame.png")), raw)
                .map_err(|error| error.to_string())?;
        }
        let capture_sha256: String = Sha256::digest(&capture)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        log(
            "desktop-published",
            json!({"read":read,"digest":candidate.digest,"marker":marker(&candidate.state),"capture_sha256":capture_sha256}),
        );
        Ok(())
    }
}

#[tauri::command]
async fn fixture_select(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
    action: Action,
) -> Result<Snapshot, String> {
    if window.label() != WINDOW {
        return Err("fixture command requires the development window".into());
    }
    host.select(&window, action).await
}

#[tauri::command]
fn fixture_waiting(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
) -> Result<Option<Uuid>, String> {
    if window.label() != WINDOW {
        return Err("fixture command requires the development window".into());
    }
    host.waiting_to_stop()
}

#[tauri::command]
fn fixture_stop(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
    operation: Uuid,
) -> Result<Option<Snapshot>, String> {
    if window.label() != WINDOW {
        return Err("fixture command requires the development window".into());
    }
    if !host.finish_read(operation, OperationTerminalReason::Cancelled)? {
        return Ok(None);
    }
    Ok(Some(host.runtime()?.snapshot()))
}

#[tauri::command]
fn fixture_continuous_snapshot(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
) -> Result<Option<Snapshot>, String> {
    if window.label() != WINDOW {
        return Err("fixture command requires the development window".into());
    }
    let runtime = host.runtime()?;
    Ok(
        (runtime.advance.is_some() || runtime.withdrawal_receipt.is_some())
            .then(|| runtime.snapshot()),
    )
}

#[tauri::command]
fn fixture_advance(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
    read: TargetReadKey,
) -> Result<bool, String> {
    if window.label() != WINDOW {
        return Err("fixture command requires the development window".into());
    }
    let mut runtime = host.runtime()?;
    if !runtime
        .advance
        .as_ref()
        .is_some_and(|next| next.read == read && next.rendered)
    {
        return Ok(false);
    }
    let next = runtime
        .advance
        .take()
        .ok_or("fixture continuation missing")?;
    next.next
        .send(())
        .map_err(|_| "fixture read continuation has ended")?;
    log("webview-advance", json!({"read":read}));
    Ok(true)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rendered {
    operation: Uuid,
    read: Option<TargetReadKey>,
    marker: Option<String>,
    has_image: bool,
}

#[tauri::command]
fn fixture_rendered(
    window: WebviewWindow,
    host: tauri::State<'_, Arc<Host>>,
    rendered: Rendered,
) -> Result<(), String> {
    if window.label() != WINDOW {
        return Err("fixture receipt requires the development window".into());
    }
    let mut runtime = host.runtime()?;
    let snapshot = runtime.snapshot();
    if snapshot.operation != Some(rendered.operation)
        || snapshot.read != rendered.read
        || snapshot.marker != rendered.marker
        || snapshot.png.is_some() != rendered.has_image
    {
        return Err("rendered snapshot is stale".into());
    }
    log(
        "webview-rendered",
        json!({"operation":rendered.operation,"stage":snapshot.stage,"has_image":rendered.has_image,"read":rendered.read,"marker":rendered.marker}),
    );
    if let Some(next) = runtime
        .advance
        .as_mut()
        .filter(|next| Some(next.read) == rendered.read)
    {
        next.rendered = true;
    }
    if snapshot.stage == LensStage::Failed {
        if let Some(receipt) = runtime.withdrawal_receipt.take() {
            let _ = receipt.send(());
        }
    }
    if let Some(active) = runtime.active_read.as_ref().filter(|active| {
        snapshot.stage == LensStage::Cancelled
            && matches!(active.action, Action::ContinuousA)
            && active.platform.validation_waiting()
    }) {
        log(
            "desktop-resuming-late-after-withdrawal",
            json!({"read":active.key}),
        );
        active.platform.deliver_validation_result();
    }
    Ok(())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let [bins, output]: [_; 2] = std::env::args_os()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "usage: lens <trusted fixture binary directory> <new evidence directory>")?;
    let bins = Path::new(&bins).canonicalize()?;
    fs::create_dir(&output)?;
    let output = Path::new(&output).canonicalize()?;
    let webview = tempfile::tempdir()?;
    let host = Host {
        bins,
        output,
        runtime: Mutex::default(),
        selection: tokio::sync::Mutex::new(()),
    };
    let profile = webview.path().to_owned();
    tauri::Builder::default()
        .manage(Arc::new(host))
        .setup(move |app| {
            WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
                .title("Lens Fixture Desktop")
                .inner_size(720.0, 600.0)
                .data_directory(profile)
                .build()?;
            log(
                "desktop-started",
                json!({"runtime":"tauri-webview2","scope":"cooperative-fixture"}),
            );
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let host = window.state::<Arc<Host>>();
                if host.selection.try_lock().is_err() {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            fixture_select,
            fixture_waiting,
            fixture_stop,
            fixture_continuous_snapshot,
            fixture_advance,
            fixture_rendered
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
