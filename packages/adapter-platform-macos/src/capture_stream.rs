#![forbid(unsafe_code)]

//! One fresh, bounded native stream per read. No cross-read sample queue exists.

use port_platform::PlatformError;
use screencapturekit::{
    async_api::AsyncSCStream,
    cm::{CMSampleBufferExt, CMSampleBufferSCExt, FrameInfo, SCFrameStatus},
    screenshot_manager::CGImageExt,
    stream::{
        configuration::SCStreamConfiguration, content_filter::SCContentFilter,
        output_type::SCStreamOutputType,
    },
};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

/// Purpose-bound debug experiments; none change the real native Stop attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidationCapture {
    HoldBeforeStop,
    InvalidAfterStop,
    CleanupUnconfirmedAfterStop,
}

#[cfg(debug_assertions)]
#[derive(Default)]
enum ValidationState {
    #[default]
    Disabled,
    Configured(SourceValidationCapture),
    Armed {
        action: SourceValidationCapture,
        operation: uuid::Uuid,
    },
    Consumed,
}

#[cfg(debug_assertions)]
impl ValidationState {
    fn configure(&mut self, action: Option<SourceValidationCapture>, metadata_enabled: bool) {
        if matches!(self, Self::Disabled) && metadata_enabled {
            if let Some(action) = action {
                *self = Self::Configured(action);
            }
        }
    }

    fn arm(&mut self, operation: uuid::Uuid) {
        if let Self::Configured(action) = self {
            *self = Self::Armed {
                action: *action,
                operation,
            };
        }
    }

    fn take(
        &mut self,
        target: port_platform::capture::CaptureTarget,
    ) -> Option<SourceValidationCapture> {
        let port_platform::capture::CaptureTarget::Registered(read) = target else {
            return None;
        };
        match self {
            Self::Armed { action, operation } if *operation == read.target.operation_id => {
                let action = *action;
                *self = Self::Consumed;
                Some(action)
            }
            _ => None,
        }
    }
}

#[cfg(debug_assertions)]
static VALIDATION: std::sync::Mutex<ValidationState> =
    std::sync::Mutex::new(ValidationState::Disabled);

#[cfg(debug_assertions)]
pub(crate) fn configure_validation(action: Option<SourceValidationCapture>) {
    if let Ok(mut state) = VALIDATION.lock() {
        state.configure(action, crate::source::source_validation_metadata_enabled());
    }
}

#[cfg(debug_assertions)]
pub(crate) fn arm_validation(operation: uuid::Uuid) {
    if let Ok(mut state) = VALIDATION.lock() {
        state.arm(operation);
    }
}

#[cfg(debug_assertions)]
fn validation_read(
    target: port_platform::capture::CaptureTarget,
) -> Option<port_platform::authority::TargetReadKey> {
    match target {
        port_platform::capture::CaptureTarget::Registered(read) => Some(read),
        port_platform::capture::CaptureTarget::Legacy { .. } => None,
    }
}

pub(crate) struct SampleFrame {
    pub bitmap: image::RgbaImage,
    pub info: FrameInfo,
}

#[derive(Debug)]
pub(crate) enum CaptureFailure {
    Unavailable(String),
    Invalid(String),
    CleanupUnconfirmed(String),
}

impl CaptureFailure {
    pub fn into_platform(self) -> PlatformError {
        match self {
            Self::Invalid(message) => PlatformError::InvalidResponse(message),
            Self::Unavailable(message) => PlatformError::Operation(message),
            Self::CleanupUnconfirmed(message) => PlatformError::CleanupUnconfirmed(message),
        }
    }
}

struct StopGuard {
    latch: Option<Arc<AtomicBool>>,
    confirmed: bool,
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        if !self.confirmed {
            if let Some(latch) = &self.latch {
                latch.store(true, Ordering::Release);
            }
        }
    }
}

pub(crate) fn acquire(
    filter: &SCContentFilter,
    configuration: &SCStreamConfiguration,
    latch: Option<Arc<AtomicBool>>,
    _target: port_platform::capture::CaptureTarget,
) -> Result<SampleFrame, CaptureFailure> {
    if latch
        .as_ref()
        .is_some_and(|latch| latch.load(Ordering::Acquire))
    {
        return Err(CaptureFailure::CleanupUnconfirmed(
            "operation already has unconfirmed native cleanup".into(),
        ));
    }
    let stream = AsyncSCStream::new(filter, configuration, 1, SCStreamOutputType::Screen)
        .map_err(|error| CaptureFailure::Unavailable(error.to_string()))?;
    let mut guard = StopGuard {
        latch,
        confirmed: false,
    };
    let mut start_completion_known = false;
    let acquired = (|| {
        let start = wait(
            stream.start_capture(),
            Duration::from_secs(5),
            "capture start",
        );
        start_completion_known = start.as_ref().is_ok_and(start_is_confirmed);
        #[cfg(debug_assertions)]
        crate::source::capture_validation_metadata(
            serde_json::json!({"stage":"start_result", "read":validation_read(_target),"completion_known":start_completion_known,"accepted":matches!(&start,Ok(Ok(())))}),
        );
        let start = start.map_err(CaptureFailure::Unavailable)?;
        start.map_err(|error| CaptureFailure::Unavailable(error.to_string()))?;
        collect(&stream, _target)
    })();
    #[cfg(debug_assertions)]
    let actual_sample_acquired = acquired.is_ok();
    #[cfg(debug_assertions)]
    let action = if acquired.is_ok() {
        VALIDATION
            .lock()
            .ok()
            .and_then(|mut state| state.take(_target))
    } else {
        None
    };
    #[cfg(debug_assertions)]
    if action == Some(SourceValidationCapture::HoldBeforeStop) {
        crate::source::capture_validation_metadata(
            serde_json::json!({"stage":"validation_hold_begin","read":validation_read(_target),"duration_ms":12000,"queue_closed":stream.is_closed()}),
        );
        // Only the admitted blocking worker sleeps; no registry/admission/main
        // queue lock is held. Queue-open witnesses are not atomic OS liveness.
        std::thread::sleep(Duration::from_secs(12));
        crate::source::capture_validation_metadata(
            serde_json::json!({"stage":"validation_hold_end","read":validation_read(_target),"queue_closed":stream.is_closed()}),
        );
    }
    // Always attempt real native Stop, including start/frame failure. Do not
    // convert cleanup failure into unavailable media or publish an acquired image.
    let cleanup = confirm_cleanup(start_completion_known, stop_and_drain(&stream, _target));
    #[cfg(debug_assertions)]
    let actual_cleanup_confirmed = cleanup.is_ok();
    #[cfg(debug_assertions)]
    let (acquired, cleanup) = report_validation_fault(action, acquired, cleanup, _target);
    guard.confirmed = cleanup.is_ok();
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"acquire_return", "read":validation_read(_target),"cleanup_confirmed":guard.confirmed,"native_cleanup_confirmed":actual_cleanup_confirmed,"sample_acquired":actual_sample_acquired,"reported_acquisition_ok":acquired.is_ok()}),
    );
    finish(acquired, cleanup)
}

#[cfg(debug_assertions)]
fn report_validation_fault<T>(
    action: Option<SourceValidationCapture>,
    acquired: Result<T, CaptureFailure>,
    cleanup: Result<(), String>,
    target: port_platform::capture::CaptureTarget,
) -> (Result<T, CaptureFailure>, Result<(), String>) {
    // Do not hide a real failure, skip native cleanup, or call this an OS error.
    if acquired.is_ok() && cleanup.is_ok() {
        let (fault, acquired, cleanup) = match action {
            Some(SourceValidationCapture::InvalidAfterStop) => (
                "invalid_after_stop",
                Err(CaptureFailure::Invalid("debug validation injected an invalid acquisition report after real native cleanup".into())),
                cleanup,
            ),
            Some(SourceValidationCapture::CleanupUnconfirmedAfterStop) => (
                "cleanup_unconfirmed_after_stop",
                acquired,
                Err("debug validation injected an unconfirmed cleanup report after real native cleanup".into()),
            ),
            _ => return (acquired, cleanup),
        };
        crate::source::capture_validation_metadata(
            serde_json::json!({"stage":"validation_injected_report","read":validation_read(target),"fault":fault,"actual_native_cleanup_confirmed":true}),
        );
        return (acquired, cleanup);
    }
    (acquired, cleanup)
}

fn start_is_confirmed(start: &Result<(), screencapturekit::error::SCError>) -> bool {
    // Pinned completion.rs uses cancel_on_drop=false; Swift Stream.swift starts
    // and stops in separate Tasks. Neither an outer timeout nor its configurable
    // completion timeout proves that a late Start cannot follow Stop acknowledgment.
    !matches!(start, Err(screencapturekit::error::SCError::CaptureStartFailed(message))
        if screencapturekit::utils::completion::is_timeout_error(message))
}

fn confirm_cleanup(
    start_completion_known: bool,
    stopped: Result<(), String>,
) -> Result<(), String> {
    stopped?;
    if start_completion_known {
        Ok(())
    } else {
        Err("native Start completion remains unconfirmed despite Stop acknowledgment".into())
    }
}

pub(crate) fn required_content(
    info: &FrameInfo,
) -> Result<port_platform::model::Bounds, CaptureFailure> {
    let (Some(rect), Some(scale)) = (info.content_rect, info.scale_factor) else {
        return Err(CaptureFailure::Invalid(
            "native sample lacks required content geometry".into(),
        ));
    };
    let bounds = port_platform::model::Bounds {
        x: rect.origin.x * scale,
        y: rect.origin.y * scale,
        width: rect.size.width * scale,
        height: rect.size.height * scale,
    };
    if !scale.is_finite() || scale <= 0.0 || !port_platform::geometry::valid_bounds(bounds) {
        return Err(CaptureFailure::Invalid(
            "native sample has invalid required content geometry".into(),
        ));
    }
    Ok(bounds)
}

fn finish<T>(
    acquired: Result<T, CaptureFailure>,
    cleanup: Result<(), String>,
) -> Result<T, CaptureFailure> {
    cleanup.map_err(|error| {
        CaptureFailure::CleanupUnconfirmed(format!(
            "native image capture cleanup is unconfirmed: {error}"
        ))
    })?;
    acquired
}

fn collect(
    stream: &AsyncSCStream,
    _target: port_platform::capture::CaptureTarget,
) -> Result<SampleFrame, CaptureFailure> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut missing_metadata = false;
    for _ in 0..24 {
        let sample = wait_until(stream.next(), deadline, "capture frame")
            .map_err(|error| end_failure(missing_metadata, error))?
            .ok_or_else(|| {
                end_failure(
                    missing_metadata,
                    format!("native image stream ended: {:?}", stream.take_error()),
                )
            })?;
        if !sample.is_valid() {
            continue;
        }
        let Some(info) = sample.frame_info() else {
            missing_metadata = true;
            continue;
        };
        if info.frame_status.is_none() {
            missing_metadata = true;
            continue;
        }
        if info.frame_status != Some(SCFrameStatus::Complete) {
            continue;
        }
        required_content(&info)?;
        let image = sample.cg_image().map_err(|error| {
            CaptureFailure::Invalid(format!("same-sample image conversion failed: {error}"))
        })?;
        let (width, height) = (image.width(), image.height());
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(CaptureFailure::Invalid(
                "native source image has invalid or unbounded pixel dimensions".into(),
            ));
        }
        let mut pixels = image
            .rgba_data()
            .map_err(|error| CaptureFailure::Invalid(error.to_string()))?;
        super::capture::unpremultiply_rgba(&mut pixels);
        let bitmap =
            image::RgbaImage::from_raw(width as u32, height as u32, pixels).ok_or_else(|| {
                CaptureFailure::Invalid("native source image has an invalid RGBA buffer".into())
            })?;
        #[cfg(debug_assertions)]
        crate::source::capture_validation_metadata(
            serde_json::json!({"stage":"sample_acquired", "read":validation_read(_target),"status":"complete", "display_time":info.display_time,"pixel_width":width,"pixel_height":height}),
        );
        return Ok(SampleFrame { bitmap, info });
    }
    Err(end_failure(
        missing_metadata,
        "native capture supplied no valid Complete sample within its sample limit".into(),
    ))
}

fn end_failure(missing_metadata: bool, detail: String) -> CaptureFailure {
    if missing_metadata {
        CaptureFailure::Invalid("native sample lacks required frame metadata".into())
    } else {
        CaptureFailure::Unavailable(detail)
    }
}

fn stop_and_drain(
    stream: &AsyncSCStream,
    _target: port_platform::capture::CaptureTarget,
) -> Result<(), String> {
    let stopped = wait(
        stream.stop_capture(),
        Duration::from_secs(5),
        "capture Stop",
    )
    .and_then(|result| result.map_err(|error| error.to_string()));
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"stop_ack", "read":validation_read(_target),"accepted":stopped.is_ok()}),
    );
    stopped?;
    let deadline = Instant::now() + Duration::from_secs(2);
    // A successful native Stop closes, but does not clear, the capacity-one
    // dependency queue. Discard bounded remaining samples without reading images.
    for _discarded in 0..=3 {
        let next = wait_until(stream.next(), deadline, "capture queue end");
        #[cfg(debug_assertions)]
        if !matches!(&next, Ok(Some(_))) {
            crate::source::capture_validation_metadata(
                serde_json::json!({"stage":"queue_end", "read":validation_read(_target),"discarded_count":_discarded,"ended":matches!(&next,Ok(None))}),
            );
        }
        if next?.is_none() {
            return Ok(());
        }
    }
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"queue_end", "read":validation_read(_target),"discarded_count":4,"ended":false}),
    );
    Err("native capture queue did not reach its end within the discard limit".into())
}

pub(crate) fn wait<F: Future>(
    future: F,
    timeout: Duration,
    name: &str,
) -> Result<F::Output, String> {
    wait_until(future, Instant::now() + timeout, name)
}

fn wait_until<F: Future>(future: F, deadline: Instant, name: &str) -> Result<F::Output, String> {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return Ok(result);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(format!("{name} timed out; completion unconfirmed"));
        };
        std::thread::park_timeout(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(debug_assertions)]
    fn registered(operation: u128) -> port_platform::capture::CaptureTarget {
        port_platform::capture::CaptureTarget::Registered(port_platform::authority::TargetReadKey {
            target: port_platform::authority::TargetAuthority {
                operation_id: uuid::Uuid::from_u128(operation),
                receipt: port_platform::authority::TargetReceipt::try_from(uuid::Uuid::from_u128(
                    2,
                ))
                .unwrap(),
            },
            sequence: port_platform::authority::ReadSequence::FIRST,
        })
    }

    #[test]
    #[cfg(debug_assertions)]
    fn validation_is_disabled_until_armed_and_bound_to_one_registered_operation() {
        let operation = uuid::Uuid::from_u128(1);
        let mut state = ValidationState::Disabled;
        state.configure(Some(SourceValidationCapture::HoldBeforeStop), false);
        state.configure(None, true);
        state.arm(operation);
        assert_eq!(state.take(registered(1)), None);
        state.configure(Some(SourceValidationCapture::HoldBeforeStop), true);
        assert_eq!(state.take(registered(1)), None);
        state.arm(operation);
        state.arm(uuid::Uuid::from_u128(99));
        state.configure(Some(SourceValidationCapture::InvalidAfterStop), true);
        assert_eq!(state.take(registered(99)), None);
        assert_eq!(
            state.take(port_platform::capture::CaptureTarget::Legacy { window_id: 1 }),
            None
        );
        assert_eq!(
            state.take(registered(1)),
            Some(SourceValidationCapture::HoldBeforeStop)
        );
        state.arm(uuid::Uuid::from_u128(99));
        state.configure(Some(SourceValidationCapture::InvalidAfterStop), true);
        assert_eq!(state.take(registered(99)), None);
        assert_eq!(state.take(registered(1)), None);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn concurrent_registered_reads_consume_the_validation_action_only_once() {
        let state = Arc::new(std::sync::Mutex::new(ValidationState::Configured(
            SourceValidationCapture::HoldBeforeStop,
        )));
        state.lock().unwrap().arm(uuid::Uuid::from_u128(1));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads = (0..2)
            .map(|_| {
                let state = state.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let consumed = state.lock().unwrap().take(registered(1)).is_some();
                    consumed
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn injected_reports_require_completed_acquisition_and_cleanup_and_preserve_real_failures() {
        for action in [
            SourceValidationCapture::InvalidAfterStop,
            SourceValidationCapture::CleanupUnconfirmedAfterStop,
        ] {
            let (acquired, cleanup) = report_validation_fault(
                Some(action),
                Ok(7),
                Err("actual Stop failure".into()),
                registered(1),
            );
            assert_eq!(acquired.unwrap(), 7);
            assert_eq!(cleanup.unwrap_err(), "actual Stop failure");
            let (acquired, cleanup) = report_validation_fault::<()>(
                Some(action),
                Err(CaptureFailure::Unavailable("actual frame failure".into())),
                Ok(()),
                registered(1),
            );
            assert!(matches!(acquired, Err(CaptureFailure::Unavailable(_))));
            assert!(cleanup.is_ok());
        }
        let (acquired, cleanup) = report_validation_fault(
            Some(SourceValidationCapture::InvalidAfterStop),
            Ok(7),
            Ok(()),
            registered(1),
        );
        assert!(matches!(
            finish(acquired, cleanup),
            Err(CaptureFailure::Invalid(_))
        ));
        let (acquired, cleanup) = report_validation_fault(
            Some(SourceValidationCapture::CleanupUnconfirmedAfterStop),
            Ok(7),
            Ok(()),
            registered(1),
        );
        let latch = Arc::new(AtomicBool::new(false));
        drop(StopGuard {
            latch: Some(latch.clone()),
            confirmed: cleanup.is_ok(),
        });
        assert!(latch.load(Ordering::Acquire));
        assert!(matches!(
            finish(acquired, cleanup),
            Err(CaptureFailure::CleanupUnconfirmed(_))
        ));
        let (acquired, cleanup) = report_validation_fault(None, Ok(7), Ok(()), registered(1));
        assert_eq!(finish(acquired, cleanup).unwrap(), 7);
    }

    #[test]
    fn required_metadata_classification_is_consistent_on_all_bounded_terminations() {
        for detail in ["deadline", "queue ended", "non-Complete sample limit"] {
            assert!(matches!(
                end_failure(true, detail.into()),
                CaptureFailure::Invalid(_)
            ));
            assert!(matches!(
                end_failure(false, detail.into()),
                CaptureFailure::Unavailable(_)
            ));
        }
        assert!(matches!(
            required_content(&FrameInfo::default()),
            Err(CaptureFailure::Invalid(_))
        ));
        let info = FrameInfo {
            content_rect: Some(Default::default()),
            scale_factor: Some(1.0),
            ..Default::default()
        };
        assert!(matches!(
            required_content(&info),
            Err(CaptureFailure::Invalid(_))
        ));
    }

    #[test]
    fn cleanup_failure_never_publishes_success_or_becomes_unavailable() {
        assert!(matches!(
            CaptureFailure::CleanupUnconfirmed("Stop unknown".into()).into_platform(),
            PlatformError::CleanupUnconfirmed(_)
        ));
        for acquired in [
            Ok(7),
            Err(CaptureFailure::Unavailable("unavailable".into())),
            Err(CaptureFailure::Invalid("invalid".into())),
        ] {
            assert!(matches!(
                finish(acquired, Err("Stop failed".into())),
                Err(CaptureFailure::CleanupUnconfirmed(_))
            ));
        }
        assert_eq!(finish(Ok(7), Ok(())).unwrap(), 7);
        assert!(matches!(
            finish::<()>(Err(CaptureFailure::Invalid("invalid".into())), Ok(())),
            Err(CaptureFailure::Invalid(_))
        ));
        assert!(confirm_cleanup(false, Ok(())).is_err());
        assert!(confirm_cleanup(true, Ok(())).is_ok());
        assert!(confirm_cleanup(true, Err("Stop failed".into())).is_err());
    }

    #[test]
    fn internal_start_timeout_is_not_a_known_native_completion() {
        use screencapturekit::error::SCError;
        assert!(start_is_confirmed(&Ok(())));
        assert!(start_is_confirmed(&Err(SCError::CaptureStartFailed(
            "rejected".into()
        ))));
        let timeout = screencapturekit::utils::completion::TIMEOUT_MESSAGE_PREFIX.to_string();
        assert!(!start_is_confirmed(&Err(SCError::CaptureStartFailed(
            timeout
        ))));
    }

    #[test]
    fn unconfirmed_guard_latches_failure_even_during_unwind_and_never_clears_it() {
        let latch = Arc::new(AtomicBool::new(false));
        let _ = std::panic::catch_unwind({
            let latch = Arc::clone(&latch);
            move || {
                let _guard = StopGuard {
                    latch: Some(latch),
                    confirmed: false,
                };
                panic!("test unwind");
            }
        });
        assert!(latch.load(Ordering::Acquire));
        drop(StopGuard {
            latch: Some(Arc::clone(&latch)),
            confirmed: true,
        });
        assert!(latch.load(Ordering::Acquire));
        let clean = Arc::new(AtomicBool::new(false));
        drop(StopGuard {
            latch: Some(Arc::clone(&clean)),
            confirmed: true,
        });
        assert!(!clean.load(Ordering::Acquire));
    }

    #[test]
    fn finite_wait_handles_ready_and_deadline_without_runtime_or_waiting_thread() {
        assert_eq!(
            wait(std::future::ready(7), Duration::ZERO, "ready").unwrap(),
            7
        );
        assert!(wait(
            std::future::pending::<()>(),
            Duration::from_millis(1),
            "test wait"
        )
        .unwrap_err()
        .contains("test wait"));
    }

    #[test]
    fn owned_completion_wakes_the_waiting_capture_thread() {
        use std::{pin::Pin, sync::Mutex};
        #[derive(Default)]
        struct State {
            ready: bool,
            waker: Option<Waker>,
        }
        struct Completion(Arc<Mutex<State>>);
        impl Future for Completion {
            type Output = u8;
            fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u8> {
                let mut state = self.0.lock().unwrap();
                if state.ready {
                    Poll::Ready(9)
                } else {
                    state.waker = Some(context.waker().clone());
                    Poll::Pending
                }
            }
        }
        let state = Arc::new(Mutex::new(State::default()));
        let producer = Arc::clone(&state);
        let thread = std::thread::spawn(move || loop {
            let waker = {
                let mut state = producer.lock().unwrap();
                if state.waker.is_some() {
                    state.ready = true;
                    state.waker.take()
                } else {
                    None
                }
            };
            if let Some(waker) = waker {
                waker.wake();
                break;
            }
            std::thread::yield_now();
        });
        let result = wait(
            Completion(state),
            Duration::from_secs(1),
            "owned completion",
        );
        thread.join().unwrap();
        assert_eq!(result.unwrap(), 9);
    }
}
