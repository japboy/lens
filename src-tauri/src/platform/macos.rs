use super::{ExtractionLimits, PlatformError};
use crate::model::{ExtractionResult, SelectedWindow, WindowIdentity, WindowPickerReply};
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_void, CStr, CString};
use std::num::NonZeroU64;
use tauri::WebviewWindow;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

type PickerCallback = unsafe extern "C" fn(*const c_char, *mut c_void);
type WindowTransitionCallback = unsafe extern "C" fn(bool, *mut c_void);
type WindowObservationCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

unsafe extern "C" {
    fn lens_accessibility_is_trusted() -> bool;
    fn lens_accessibility_request_trust() -> bool;
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
    fn lens_present_window_picker_for_operation(
        operation_id: *const c_char,
        callback: PickerCallback,
        context: *mut c_void,
    ) -> bool;
    fn lens_extract_window_json(
        pid: i32,
        selected_title: *const c_char,
        application_name: *const c_char,
        selected_x: f64,
        selected_y: f64,
        selected_width: f64,
        selected_height: f64,
        max_nodes: u32,
        max_text_bytes: u32,
        max_resource_refs: u32,
        max_resource_uri_bytes: u32,
        max_total_resource_uri_bytes: u32,
    ) -> *mut c_char;
    fn lens_extract_registered_window_json(
        operation_id: *const c_char,
        window_id: u32,
        max_nodes: u32,
        max_text_bytes: u32,
        max_resource_refs: u32,
        max_resource_uri_bytes: u32,
        max_total_resource_uri_bytes: u32,
    ) -> *mut c_char;
    fn lens_start_window_observation_json(
        operation_id: *const c_char,
        context_id: *const c_char,
        source_registration_id: *const c_char,
        observer_epoch: u64,
        window_id: u32,
        callback: WindowObservationCallback,
        context: *mut c_void,
        started_out: *mut bool,
    ) -> *mut c_char;
    fn lens_stop_window_observation(
        operation_id: *const c_char,
        source_registration_id: *const c_char,
    ) -> bool;
    fn lens_release_registered_window(operation_id: *const c_char, window_id: u32) -> bool;
    fn lens_release_window_operation(operation_id: *const c_char) -> bool;
    fn lens_free_string(value: *mut c_char);
}

struct PickerContext {
    sender: Option<oneshot::Sender<String>>,
}

struct WindowTransitionContext {
    sender: Option<oneshot::Sender<bool>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowObservationNotification {
    WindowTitleChanged,
    WindowMoved,
    WindowResized,
    WindowDestroyed,
    ApplicationChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowObservationEvent {
    pub operation_id: Uuid,
    pub context_id: Uuid,
    pub source_registration_id: Uuid,
    pub observer_epoch: NonZeroU64,
    pub window_id: u32,
    pub notification: WindowObservationNotification,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum NativeObservationStartReply {
    Started {
        registered_notifications: Vec<WindowObservationNotification>,
        diagnostics: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowObservationStart {
    pub registered_notifications: Vec<WindowObservationNotification>,
    pub diagnostics: Vec<String>,
}

pub type WindowObservationReceiver = mpsc::Receiver<WindowObservationEvent>;

struct WindowObservationCallbackContext {
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    window_id: u32,
    sender: mpsc::Sender<WindowObservationEvent>,
}

pub struct WindowObservationRegistration {
    operation_id: Uuid,
    source_registration_id: Uuid,
    callback_context: Option<Box<WindowObservationCallbackContext>>,
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

unsafe extern "C" fn window_observation_callback(json: *const c_char, context: *mut c_void) {
    if json.is_null() || context.is_null() {
        return;
    }
    // SAFETY: The boxed context is owned by `WindowObservationRegistration` and remains at a
    // stable address until native stop has synchronously removed the main-run-loop observer.
    let context = unsafe { &*context.cast::<WindowObservationCallbackContext>() };
    // SAFETY: The native bridge keeps this NUL-terminated JSON buffer alive for this callback.
    let Ok(json) = unsafe { CStr::from_ptr(json) }.to_str() else {
        return;
    };
    let Ok(event) = serde_json::from_str::<WindowObservationEvent>(json) else {
        return;
    };
    if event.operation_id != context.operation_id
        || event.context_id != context.context_id
        || event.source_registration_id != context.source_registration_id
        || event.observer_epoch != context.observer_epoch
        || event.window_id != context.window_id
    {
        return;
    }
    // The per-source channel has capacity one. A full channel means an invalidation is already
    // pending and this noisy native signal is intentionally coalesced at the FFI boundary.
    let _ = context.sender.try_send(event);
}

impl WindowObservationRegistration {
    fn stop_inner(&mut self) -> Result<(), PlatformError> {
        if self.callback_context.is_none() {
            return Ok(());
        }
        let operation_id = CString::new(self.operation_id.to_string())
            .expect("UUID text contains no interior NUL");
        let source_registration_id = CString::new(self.source_registration_id.to_string())
            .expect("UUID text contains no interior NUL");
        // SAFETY: Both strings remain valid for this synchronous main-run-loop teardown. Native
        // clears its callback pointer and refcon before this method drops the boxed context.
        let stopped = unsafe {
            lens_stop_window_observation(operation_id.as_ptr(), source_registration_id.as_ptr())
        };
        if !stopped {
            return Err(PlatformError::Operation(
                "native source observation was unavailable during stop".into(),
            ));
        }
        self.callback_context = None;
        Ok(())
    }
}

impl Drop for WindowObservationRegistration {
    fn drop(&mut self) {
        let _ = self.stop_inner();
        // If native reported an already-absent source, it cannot retain the callback context.
        self.callback_context = None;
    }
}

pub fn accessibility_is_trusted() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrusted.
    unsafe { lens_accessibility_is_trusted() }
}

pub fn request_accessibility_trust() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrustedWithOptions.
    unsafe { lens_accessibility_request_trust() }
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

pub async fn present_window_picker_for_operation(
    operation_id: Uuid,
) -> Result<WindowPickerReply, PlatformError> {
    present_window_picker_impl(operation_id).await
}

async fn present_window_picker_impl(
    operation_id: Uuid,
) -> Result<WindowPickerReply, PlatformError> {
    let (sender, receiver) = oneshot::channel();
    let context = Box::new(PickerContext {
        sender: Some(sender),
    });
    let raw_context = Box::into_raw(context).cast::<c_void>();

    // SAFETY: The context remains owned by the callback when presentation succeeds. If
    // presentation fails synchronously, it is reconstructed below.
    let operation_id =
        CString::new(operation_id.to_string()).expect("UUID text contains no interior NUL");
    // SAFETY: The UUID and callback context remain valid for the synchronous presentation
    // request. A successful request transfers callback-context ownership to native.
    let presented = unsafe {
        lens_present_window_picker_for_operation(
            operation_id.as_ptr(),
            picker_callback,
            raw_context,
        )
    };
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

pub fn extract_window(
    target: &SelectedWindow,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    let title = CString::new(target.facts.title.as_str())
        .map_err(|_| PlatformError::Operation("window title contains an interior NUL".into()))?;
    let application_name = CString::new(target.facts.application_name.as_str()).map_err(|_| {
        PlatformError::Operation("application name contains an interior NUL".into())
    })?;
    // SAFETY: All pointer arguments are valid for the duration of the call. The native bridge
    // returns a malloc-owned NUL-terminated buffer, released with its matching free function.
    let raw = unsafe {
        lens_extract_window_json(
            target.identity.pid,
            title.as_ptr(),
            application_name.as_ptr(),
            target.facts.frame.x,
            target.facts.frame.y,
            target.facts.frame.width,
            target.facts.frame.height,
            limits.max_nodes,
            limits.max_text_bytes,
            limits.max_resource_refs,
            limits.max_resource_uri_bytes,
            limits.max_total_resource_uri_bytes,
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
    unsafe { lens_free_string(raw) };
    serde_json::from_str(&json)
        .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))
}

pub fn extract_registered_window(
    operation_id: Uuid,
    identity: &WindowIdentity,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    let operation_id =
        CString::new(operation_id.to_string()).expect("UUID text contains no interior NUL");
    // SAFETY: The UUID remains valid for this blocking call. Native traverses only the AXWindow
    // retained under the exact operation/window identity and returns a malloc-owned JSON buffer.
    let raw = unsafe {
        lens_extract_registered_window_json(
            operation_id.as_ptr(),
            identity.window_id,
            limits.max_nodes,
            limits.max_text_bytes,
            limits.max_resource_refs,
            limits.max_resource_uri_bytes,
            limits.max_total_resource_uri_bytes,
        )
    };
    if raw.is_null() {
        return Err(PlatformError::Operation(
            "registered native Accessibility extraction returned no data".into(),
        ));
    }
    // SAFETY: Native returned one valid NUL-terminated malloc-owned string.
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: This is the matching release for the native JSON response.
    unsafe { lens_free_string(raw) };
    serde_json::from_str(&json)
        .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))
}

pub fn start_window_observation(
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    identity: &WindowIdentity,
) -> Result<
    (
        WindowObservationRegistration,
        WindowObservationReceiver,
        WindowObservationStart,
    ),
    PlatformError,
> {
    let operation_id_text =
        CString::new(operation_id.to_string()).expect("UUID text contains no interior NUL");
    let context_id_text =
        CString::new(context_id.to_string()).expect("UUID text contains no interior NUL");
    let source_registration_id_text = CString::new(source_registration_id.to_string())
        .expect("UUID text contains no interior NUL");
    let (sender, receiver) = mpsc::channel(1);
    let mut callback_context = Box::new(WindowObservationCallbackContext {
        operation_id,
        context_id,
        source_registration_id,
        observer_epoch,
        window_id: identity.window_id,
        sender,
    });
    let raw_context = (&mut *callback_context as *mut WindowObservationCallbackContext).cast();
    let mut started = false;
    // SAFETY: The boxed context remains stable and Rust-owned until synchronous native stop.
    // Native installs every AXObserver registration and its run-loop source before returning.
    let raw = unsafe {
        lens_start_window_observation_json(
            operation_id_text.as_ptr(),
            context_id_text.as_ptr(),
            source_registration_id_text.as_ptr(),
            observer_epoch.get(),
            identity.window_id,
            window_observation_callback,
            raw_context,
            &mut started,
        )
    };
    if raw.is_null() {
        return Err(PlatformError::Operation(
            "native source observation returned no start result".into(),
        ));
    }
    // SAFETY: Native returned one valid NUL-terminated malloc-owned string.
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: This is the matching release for the native JSON response.
    unsafe { lens_free_string(raw) };
    let reply = match serde_json::from_str::<NativeObservationStartReply>(&json) {
        Ok(reply) => reply,
        Err(error) => {
            if started {
                // SAFETY: `started` is written by this exact native invocation. The UUIDs remain
                // valid while native synchronously clears the callback and refcon on main.
                unsafe {
                    lens_stop_window_observation(
                        operation_id_text.as_ptr(),
                        source_registration_id_text.as_ptr(),
                    )
                };
            }
            return Err(PlatformError::InvalidResponse(format!(
                "{error}; response={json}"
            )));
        }
    };
    let start = match reply {
        NativeObservationStartReply::Started {
            registered_notifications,
            diagnostics,
        } => WindowObservationStart {
            registered_notifications,
            diagnostics,
        },
        NativeObservationStartReply::Error { message } => {
            return Err(PlatformError::Operation(message));
        }
    };
    if !started {
        return Err(PlatformError::InvalidResponse(
            "native observation reported started JSON without transferring ownership".into(),
        ));
    }
    let registration = WindowObservationRegistration {
        operation_id,
        source_registration_id,
        callback_context: Some(callback_context),
    };
    Ok((registration, receiver, start))
}

pub fn release_registered_window(operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
    let operation_id =
        CString::new(operation_id.to_string()).expect("UUID text contains no interior NUL");
    // SAFETY: The UUID is valid for this synchronous main-thread-confined registry mutation.
    if unsafe { lens_release_registered_window(operation_id.as_ptr(), window_id) } {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "native selected-window registration is unavailable".into(),
        ))
    }
}

pub fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
    let operation_id =
        CString::new(operation_id.to_string()).expect("UUID text contains no interior NUL");
    // SAFETY: The UUID is valid for this synchronous main-thread-confined registry mutation.
    if unsafe { lens_release_window_operation(operation_id.as_ptr()) } {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "native selected-window operation is unavailable".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_event_requires_closed_notification_and_nonzero_epoch() {
        let event: WindowObservationEvent = serde_json::from_str(
            r#"{
                "operation_id":"00000000-0000-0000-0000-000000000001",
                "context_id":"00000000-0000-0000-0000-000000000002",
                "source_registration_id":"00000000-0000-0000-0000-000000000003",
                "observer_epoch":4,
                "window_id":17,
                "notification":"window_title_changed"
            }"#,
        )
        .expect("valid event");
        assert_eq!(event.observer_epoch.get(), 4);
        assert_eq!(
            event.notification,
            WindowObservationNotification::WindowTitleChanged
        );

        let invalid = serde_json::from_str::<WindowObservationEvent>(
            r#"{
                "operation_id":"00000000-0000-0000-0000-000000000001",
                "context_id":"00000000-0000-0000-0000-000000000002",
                "source_registration_id":"00000000-0000-0000-0000-000000000003",
                "observer_epoch":0,
                "window_id":17,
                "notification":"unbounded_native_detail"
            }"#,
        );
        assert!(invalid.is_err());
    }

    #[test]
    fn observation_handle_and_receiver_can_move_to_the_live_source_actor() {
        fn assert_send<T: Send>() {}

        assert_send::<WindowObservationRegistration>();
        assert_send::<WindowObservationReceiver>();
    }
}
