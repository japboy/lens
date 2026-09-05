use port_platform::PlatformError;
use std::ffi::c_void;
use tauri::WebviewWindow;
use tokio::sync::oneshot;

type WindowTransitionCallback = unsafe extern "C" fn(bool, *mut c_void);

unsafe extern "C" {
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

pub fn present_window_from_screen_right(window: &WebviewWindow) -> Result<(), PlatformError> {
    let native_window = window.ns_window().map_err(|error| {
        PlatformError::Operation(format!("unable to resolve native Preview window: {error}"))
    })?;
    // SAFETY: Tauri owns `native_window` for at least the lifetime of `window`. The bridge uses
    // the pointer synchronously on the AppKit main queue and retains no raw pointer afterward.
    let presented = unsafe { lens_present_window_from_screen_right(native_window) };
    if presented {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "unable to present Preview from the right side of its screen".into(),
        ))
    }
}

pub async fn dismiss_window_to_screen_right(window: &WebviewWindow) -> Result<(), PlatformError> {
    let native_window = window.ns_window().map_err(|error| {
        PlatformError::Operation(format!("unable to resolve native Preview window: {error}"))
    })?;
    let (sender, receiver) = oneshot::channel();
    let context = Box::new(WindowTransitionContext {
        sender: Some(sender),
    });
    let raw_context = Box::into_raw(context).cast::<c_void>();
    // SAFETY: Tauri owns `native_window` while this future retains `window`. The context remains
    // callback-owned when dismissal starts and is reconstructed here only on synchronous refusal.
    let started = unsafe {
        lens_dismiss_window_to_screen_right(native_window, window_transition_callback, raw_context)
    };
    if !started {
        // SAFETY: A synchronous refusal guarantees that the native bridge did not retain or call
        // the callback context.
        unsafe { drop(Box::from_raw(raw_context.cast::<WindowTransitionContext>())) };
        return Err(PlatformError::Operation(
            "unable to start Preview dismissal".into(),
        ));
    }
    if receiver
        .await
        .map_err(|_| PlatformError::Operation("Preview dismissal callback was dropped".into()))?
    {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "unable to dismiss Preview to the right side of its screen".into(),
        ))
    }
}

pub async fn transition_window_frame(
    window: &WebviewWindow,
    target_x: f64,
    target_y: f64,
    target_content_width: f64,
    target_content_height: f64,
) -> Result<(), PlatformError> {
    let native_window = window.ns_window().map_err(|error| {
        PlatformError::Operation(format!("unable to resolve native Preview window: {error}"))
    })?;
    let scale_factor = window.scale_factor().map_err(|error| {
        PlatformError::Operation(format!("unable to resolve Preview scale factor: {error}"))
    })?;
    let current_position = window
        .outer_position()
        .map_err(|error| {
            PlatformError::Operation(format!("unable to resolve Preview position: {error}"))
        })?
        .to_logical::<f64>(scale_factor);
    let (sender, receiver) = oneshot::channel();
    let context = Box::new(WindowTransitionContext {
        sender: Some(sender),
    });
    let raw_context = Box::into_raw(context).cast::<c_void>();
    // SAFETY: Tauri owns `native_window` while this future retains `window`. Position deltas and
    // content dimensions are finite logical points. The callback exclusively owns `raw_context`
    // after the native transition starts.
    let started = unsafe {
        lens_transition_window_frame(
            native_window,
            target_x - current_position.x,
            target_y - current_position.y,
            target_content_width,
            target_content_height,
            window_transition_callback,
            raw_context,
        )
    };
    if !started {
        // SAFETY: A synchronous refusal guarantees that the native bridge did not retain or call
        // the callback context.
        unsafe { drop(Box::from_raw(raw_context.cast::<WindowTransitionContext>())) };
        return Err(PlatformError::Operation(
            "unable to start Preview frame transition".into(),
        ));
    }
    if receiver
        .await
        .map_err(|_| PlatformError::Operation("Preview frame callback was dropped".into()))?
    {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "unable to complete Preview frame transition".into(),
        ))
    }
}
