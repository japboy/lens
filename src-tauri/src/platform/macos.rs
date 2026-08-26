use super::PlatformError;
use crate::model::{Bounds, ExtractionResult, SelectedWindow, WindowPickerReply};
use serde::Deserialize;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    sync::Mutex,
};
use tokio::sync::{mpsc, oneshot};

type PickerCallback = unsafe extern "C" fn(*const c_char, *mut c_void);
type WindowObserverCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

unsafe extern "C" {
    fn pl_accessibility_is_trusted() -> bool;
    fn pl_accessibility_request_trust() -> bool;
    fn pl_present_window_picker(callback: PickerCallback, context: *mut c_void) -> bool;
    fn pl_copy_window_frame_json(window_id: u32) -> *mut c_char;
    fn pl_start_window_observer(
        pid: i32,
        selected_title: *const c_char,
        selected_x: f64,
        selected_y: f64,
        selected_width: f64,
        selected_height: f64,
        callback: WindowObserverCallback,
        context: *mut c_void,
    ) -> bool;
    fn pl_stop_window_observer() -> *mut c_void;
    fn pl_stop_window_observer_if_context(expected_context: *mut c_void) -> *mut c_void;
    fn pl_extract_window_json(
        pid: i32,
        selected_title: *const c_char,
        selected_x: f64,
        selected_y: f64,
        selected_width: f64,
        selected_height: f64,
        max_nodes: u32,
        max_text_bytes: u32,
    ) -> *mut c_char;
    fn pl_free_string(value: *mut c_char);
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WindowFrameReply {
    Available { frame: Bounds },
    Unavailable,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowObserverEvent {
    FrameChanged { frame: Bounds },
    Destroyed,
}

struct WindowObserverContext {
    sender: mpsc::UnboundedSender<WindowObserverEvent>,
}

static WINDOW_OBSERVER_FFI_LOCK: Mutex<()> = Mutex::new(());

pub struct WindowObserver {
    receiver: mpsc::UnboundedReceiver<WindowObserverEvent>,
    context: usize,
}

impl WindowObserver {
    pub async fn recv(&mut self) -> Option<WindowObserverEvent> {
        self.receiver.recv().await
    }
}

impl Drop for WindowObserver {
    fn drop(&mut self) {
        let Ok(_guard) = WINDOW_OBSERVER_FFI_LOCK.lock() else {
            return;
        };
        let context = self.context as *mut c_void;
        // SAFETY: The native coordinator returns this context only if it still owns the exact
        // observer created for this guard. Main-run-loop serialization prevents later callbacks.
        let reclaimed = unsafe { pl_stop_window_observer_if_context(context) };
        if reclaimed == context {
            // SAFETY: This pointer originated from Box::into_raw in `observe_window`, has not
            // been reclaimed by a replacement observer, and the native side no longer uses it.
            unsafe { drop(Box::from_raw(reclaimed.cast::<WindowObserverContext>())) };
        }
    }
}

unsafe extern "C" fn window_observer_callback(json: *const c_char, context: *mut c_void) {
    if json.is_null() || context.is_null() {
        return;
    }
    // SAFETY: The context remains Box-owned until the main-run-loop observer is synchronously
    // removed. This callback only borrows it for the duration of the call.
    let context = unsafe { &*context.cast::<WindowObserverContext>() };
    // SAFETY: The native bridge keeps the NUL-terminated callback buffer alive for this call.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    if let Ok(event) = serde_json::from_str::<WindowObserverEvent>(&json) {
        let _ = context.sender.send(event);
    }
}

struct PickerContext {
    sender: Option<oneshot::Sender<String>>,
}

unsafe extern "C" fn picker_callback(json: *const c_char, context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // SAFETY: `context` is created by `Box::into_raw` immediately before presenting the
    // picker. The native coordinator invokes exactly one terminal callback and clears it.
    let mut context = unsafe { Box::from_raw(context.cast::<PickerContext>()) };
    let response = if json.is_null() {
        r#"{"status":"error","message":"Native picker returned an empty response."}"#.into()
    } else {
        // SAFETY: The native bridge keeps the NUL-terminated buffer alive for this callback.
        unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned()
    };
    if let Some(sender) = context.sender.take() {
        let _ = sender.send(response);
    }
}

pub fn accessibility_is_trusted() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrusted.
    unsafe { pl_accessibility_is_trusted() }
}

pub fn request_accessibility_trust() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrustedWithOptions.
    unsafe { pl_accessibility_request_trust() }
}

pub async fn present_window_picker() -> Result<WindowPickerReply, PlatformError> {
    let (sender, receiver) = oneshot::channel();
    let context = Box::new(PickerContext {
        sender: Some(sender),
    });
    let raw_context = Box::into_raw(context).cast::<c_void>();

    // SAFETY: The context remains owned by the callback when presentation succeeds. If
    // presentation fails synchronously, it is reconstructed below.
    let presented = unsafe { pl_present_window_picker(picker_callback, raw_context) };
    if !presented {
        // SAFETY: A failed presentation guarantees that the native side did not retain or call
        // the callback context.
        unsafe { drop(Box::from_raw(raw_context.cast::<PickerContext>())) };
        return Err(PlatformError::PickerBusy);
    }

    let json = receiver
        .await
        .map_err(|_| PlatformError::PickerCallbackDropped)?;
    serde_json::from_str(&json)
        .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))
}

pub fn extract_window(target: &SelectedWindow) -> Result<ExtractionResult, PlatformError> {
    let title = CString::new(target.title.as_str())
        .map_err(|_| PlatformError::Operation("window title contains an interior NUL".into()))?;
    // SAFETY: All pointer arguments are valid for the duration of the call. The native bridge
    // returns a malloc-owned NUL-terminated buffer, released with its matching free function.
    let raw = unsafe {
        pl_extract_window_json(
            target.pid,
            title.as_ptr(),
            target.frame.x,
            target.frame.y,
            target.frame.width,
            target.frame.height,
            30_000,
            1_000_000,
        )
    };
    if raw.is_null() {
        return Err(PlatformError::Operation(
            "native Accessibility extraction returned no data".into(),
        ));
    }
    // SAFETY: `raw` is a valid NUL-terminated string returned by the native bridge.
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: The buffer was allocated by `PLCopyJSONString` and has not been freed yet.
    unsafe { pl_free_string(raw) };
    serde_json::from_str(&json)
        .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))
}

pub fn current_window_frame(window_id: u32) -> Result<Option<Bounds>, PlatformError> {
    // SAFETY: The native bridge receives a value-only CGWindowID and returns a malloc-owned
    // NUL-terminated JSON buffer, released below with its matching free function.
    let raw = unsafe { pl_copy_window_frame_json(window_id) };
    if raw.is_null() {
        return Err(PlatformError::Operation(
            "native window-frame lookup returned no data".into(),
        ));
    }
    // SAFETY: `raw` is a valid NUL-terminated string returned by the native bridge.
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: The buffer was allocated by `PLCopyJSONString` and has not been freed yet.
    unsafe { pl_free_string(raw) };
    let reply: WindowFrameReply = serde_json::from_str(&json)
        .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))?;
    Ok(match reply {
        WindowFrameReply::Available { frame } => Some(frame),
        WindowFrameReply::Unavailable => None,
    })
}

pub fn observe_window(target: &SelectedWindow) -> Result<WindowObserver, PlatformError> {
    let title = CString::new(target.title.as_str())
        .map_err(|_| PlatformError::Operation("window title contains an interior NUL".into()))?;
    let _guard = WINDOW_OBSERVER_FFI_LOCK
        .lock()
        .map_err(|_| PlatformError::Operation("window observer lock is poisoned".into()))?;

    // SAFETY: Stopping is synchronous on the native main run loop. Any returned pointer is the
    // unique context previously transferred by `Box::into_raw` below.
    let previous = unsafe { pl_stop_window_observer() };
    if !previous.is_null() {
        // SAFETY: The native observer has stopped and relinquished its only reference.
        unsafe { drop(Box::from_raw(previous.cast::<WindowObserverContext>())) };
    }

    let (sender, receiver) = mpsc::unbounded_channel();
    let context = Box::into_raw(Box::new(WindowObserverContext { sender })).cast::<c_void>();
    // SAFETY: All arguments remain valid for the synchronous start call. On success the native
    // observer borrows `context` until one of the synchronous stop functions returns it.
    let started = unsafe {
        pl_start_window_observer(
            target.pid,
            title.as_ptr(),
            target.frame.x,
            target.frame.y,
            target.frame.width,
            target.frame.height,
            window_observer_callback,
            context,
        )
    };
    if !started {
        // SAFETY: A failed start does not retain or use the context.
        unsafe { drop(Box::from_raw(context.cast::<WindowObserverContext>())) };
        return Err(PlatformError::Operation(
            "the selected AXWindow does not expose usable geometry notifications".into(),
        ));
    }
    Ok(WindowObserver {
        receiver,
        context: context as usize,
    })
}
