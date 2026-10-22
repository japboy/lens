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
            serde_json::json!({"stage":"start_result", "completion_known":start_completion_known,"accepted":matches!(&start,Ok(Ok(())))}),
        );
        let start = start.map_err(CaptureFailure::Unavailable)?;
        start.map_err(|error| CaptureFailure::Unavailable(error.to_string()))?;
        collect(&stream)
    })();
    // Always attempt real native Stop, including start/frame failure. Do not
    // convert cleanup failure into unavailable media or publish an acquired image.
    let cleanup = confirm_cleanup(start_completion_known, stop_and_drain(&stream));
    guard.confirmed = cleanup.is_ok();
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"acquire_return", "cleanup_confirmed":guard.confirmed,"sample_acquired":acquired.is_ok()}),
    );
    finish(acquired, cleanup)
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

fn collect(stream: &AsyncSCStream) -> Result<SampleFrame, CaptureFailure> {
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
            serde_json::json!({"stage":"sample_acquired", "status":"complete", "display_time":info.display_time,"pixel_width":width,"pixel_height":height}),
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

fn stop_and_drain(stream: &AsyncSCStream) -> Result<(), String> {
    let stopped = wait(
        stream.stop_capture(),
        Duration::from_secs(5),
        "capture Stop",
    )
    .and_then(|result| result.map_err(|error| error.to_string()));
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"stop_ack", "accepted":stopped.is_ok()}),
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
                serde_json::json!({"stage":"queue_end", "discarded_count":_discarded,"ended":matches!(&next,Ok(None))}),
            );
        }
        if next?.is_none() {
            return Ok(());
        }
    }
    #[cfg(debug_assertions)]
    crate::source::capture_validation_metadata(
        serde_json::json!({"stage":"queue_end", "discarded_count":4,"ended":false}),
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
