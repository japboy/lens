//! Finite AppKit effects. Placement and borrowed handle acquisition belong to the shell.
use port_platform::PlatformError;
use std::ffi::c_void;
use tokio::sync::oneshot;

type WindowTransitionCallback = unsafe extern "C" fn(bool, *mut c_void);

unsafe extern "C" {
    fn lens_window_background_rgba(rgba: *mut u8) -> bool;
    fn lens_dismiss_window_to_screen_right(
        window: *mut c_void,
        callback: WindowTransitionCallback,
        context: *mut c_void,
    ) -> bool;
    fn lens_present_window_from_screen_right(window: *mut c_void) -> bool;
    fn lens_transition_window_frame(
        window: *mut c_void,
        top_left_delta_x: f64,
        top_left_delta_y: f64,
        content_width: f64,
        content_height: f64,
        callback: WindowTransitionCallback,
        context: *mut c_void,
    ) -> bool;
}

/// Resolve the system window background in the current application appearance.
///
/// # Safety
/// Call on the AppKit main thread after NSApplication initialization.
pub unsafe fn window_background_rgba() -> Result<[u8; 4], PlatformError> {
    let mut rgba = [0; 4];
    // SAFETY: The caller guarantees AppKit affinity; the bridge writes exactly four bytes.
    if unsafe { lens_window_background_rgba(rgba.as_mut_ptr()) } {
        Ok(rgba)
    } else {
        Err(PlatformError::Operation(
            "unable to resolve system window background".into(),
        ))
    }
}

struct WindowTransitionContext {
    sender: Option<oneshot::Sender<bool>>,
}

unsafe extern "C" fn window_transition_callback(completed: bool, context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // SAFETY: `context` is created immediately before a native frame transition begins. The native
    // bridge invokes this callback exactly once after reaching the terminal Preview frame.
    let mut context = unsafe { Box::from_raw(context.cast::<WindowTransitionContext>()) };
    if let Some(sender) = context.sender.take() {
        let _ = sender.send(completed);
    }
}

#[derive(Clone, Copy)]
enum TransitionKind {
    Dismissal,
    Frame,
}

impl TransitionKind {
    fn refused(self) -> &'static str {
        match self {
            Self::Dismissal => "unable to start Preview dismissal",
            Self::Frame => "unable to start Preview frame transition",
        }
    }
    fn dropped(self) -> &'static str {
        match self {
            Self::Dismissal => "Preview dismissal callback was dropped",
            Self::Frame => "Preview frame callback was dropped",
        }
    }
    fn failed(self) -> &'static str {
        match self {
            Self::Dismissal => "unable to dismiss Preview to the right side of its screen",
            Self::Frame => "unable to complete Preview frame transition",
        }
    }
}

/// Handle-free completion receipt. Dropping it does not cancel AppKit work: the native
/// callback still owns and eventually reclaims its context.
pub struct WindowTransition {
    receiver: Result<oneshot::Receiver<bool>, PlatformError>,
    kind: TransitionKind,
}

impl WindowTransition {
    // start must obey exactly-once callback ownership on acceptance, no callback on refusal.
    fn start(kind: TransitionKind, start: impl FnOnce(*mut c_void) -> bool) -> Self {
        let (sender, receiver) = oneshot::channel();
        let context = Box::into_raw(Box::new(WindowTransitionContext {
            sender: Some(sender),
        }))
        .cast::<c_void>();
        let receiver = if start(context) {
            Ok(receiver)
        } else {
            // SAFETY: Synchronous refusal guarantees the bridge neither retained nor called
            // the callback. Rust still exclusively owns this allocation.
            unsafe { drop(Box::from_raw(context.cast::<WindowTransitionContext>())) };
            Err(PlatformError::Operation(kind.refused().into()))
        };
        Self { receiver, kind }
    }

    pub async fn wait(self) -> Result<(), PlatformError> {
        if self
            .receiver?
            .await
            .map_err(|_| PlatformError::Operation(self.kind.dropped().into()))?
        {
            Ok(())
        } else {
            Err(PlatformError::Operation(self.kind.failed().into()))
        }
    }
}

/// Present a hidden Preview from the right edge of its screen.
///
/// # Safety
/// Call on the AppKit main thread with a valid, live NSWindow borrowed for this call.
/// The bridge uses it synchronously; Rust never takes ownership of the window.
pub unsafe fn present_window_from_screen_right(window: *mut c_void) -> Result<(), PlatformError> {
    // SAFETY: The caller supplies a live NSWindow on its owning thread.
    if unsafe { lens_present_window_from_screen_right(window) } {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "unable to present Preview from the right side of its screen".into(),
        ))
    }
}

/// Start dismissal and return its handle-free completion receipt.
///
/// # Safety
/// Call on the AppKit main thread with a valid, live NSWindow borrowed for this call.
/// Native code establishes a strong reference before returning; its animation completion
/// block retains that reference. Never enqueue an unretained pointer from another thread.
pub unsafe fn dismiss_window_to_screen_right(window: *mut c_void) -> WindowTransition {
    WindowTransition::start(TransitionKind::Dismissal, |context| {
        // SAFETY: Main-thread entry establishes retention synchronously. Acceptance transfers
        // context ownership to exactly one callback, including immediate completion.
        unsafe { lens_dismiss_window_to_screen_right(window, window_transition_callback, context) }
    })
}

/// Start one frame transition in logical points. Native code rejects invalid geometry.
///
/// # Safety
/// The same main-thread, live borrowed NSWindow contract as dismissal applies. The bridge
/// retains the window through completion; the returned receipt contains no native handle.
pub unsafe fn transition_window_frame(
    window: *mut c_void,
    top_left_delta_x: f64,
    top_left_delta_y: f64,
    content_width: f64,
    content_height: f64,
) -> WindowTransition {
    WindowTransition::start(TransitionKind::Frame, |context| {
        // SAFETY: The caller guarantees the window contract. Native code validates geometry
        // before accepting the exclusively owned callback context.
        unsafe {
            lens_transition_window_frame(
                window,
                top_left_delta_x,
                top_left_delta_y,
                content_width,
                content_height,
                window_transition_callback,
                context,
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    fn ready(transition: WindowTransition) -> Result<(), PlatformError> {
        let mut future = std::pin::pin!(transition.wait());
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("fixture completion must already be available"),
        }
    }

    #[test]
    fn completion_receipt_and_wait_are_send() {
        fn assert_send<T: Send>(_: T) {}
        assert_send(WindowTransition::start(TransitionKind::Frame, |_| false).wait());
    }

    #[test]
    fn immediate_completion_preserves_success_and_failure() {
        for completed in [true, false] {
            for kind in [TransitionKind::Dismissal, TransitionKind::Frame] {
                let transition = WindowTransition::start(kind, |context| {
                    // SAFETY: This fixture accepts ownership and invokes the sole callback.
                    unsafe { window_transition_callback(completed, context) };
                    true
                });
                match ready(transition) {
                    Ok(()) => assert!(completed),
                    Err(PlatformError::Operation(message)) => {
                        assert!(!completed);
                        assert_eq!(message, kind.failed());
                    }
                    Err(error) => panic!("unexpected completion error: {error}"),
                }
            }
        }
    }

    #[test]
    fn refusal_retains_operation_error() {
        for kind in [TransitionKind::Dismissal, TransitionKind::Frame] {
            let transition = WindowTransition::start(kind, |_| false);
            assert!(
                matches!(ready(transition), Err(PlatformError::Operation(message)) if message == kind.refused())
            );
        }
    }

    #[test]
    fn delayed_callback_remains_owner_after_receipt_is_dropped() {
        let mut callback_context = std::ptr::null_mut();
        let transition = WindowTransition::start(TransitionKind::Dismissal, |context| {
            callback_context = context;
            true
        });
        drop(transition);
        // SAFETY: Accepted context remains callback-owned after receiver cancellation.
        unsafe { window_transition_callback(true, callback_context) };
    }

    #[test]
    fn null_callback_context_is_ignored() {
        // SAFETY: The callback explicitly accepts null as a no-op.
        unsafe { window_transition_callback(false, std::ptr::null_mut()) };
    }

    #[test]
    fn callback_loss_is_distinct_from_native_failure() {
        for kind in [TransitionKind::Dismissal, TransitionKind::Frame] {
            let (sender, receiver) = oneshot::channel();
            drop(sender);
            let transition = WindowTransition {
                receiver: Ok(receiver),
                kind,
            };
            assert!(
                matches!(ready(transition), Err(PlatformError::Operation(message)) if message == kind.dropped())
            );
        }
    }

    #[test]
    fn pending_receipt_completes_only_when_callback_arrives() {
        let mut callback_context = std::ptr::null_mut();
        let transition = WindowTransition::start(TransitionKind::Frame, |context| {
            callback_context = context;
            true
        });
        let mut future = std::pin::pin!(transition.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());
        // SAFETY: This fixture delivers the single accepted callback.
        unsafe { window_transition_callback(true, callback_context) };
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Ok(()))
        ));
    }
}
