use crate::MacOsPlatform;
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    model::{ExtractionResult, SelectedWindow, WindowIdentity},
    observation::{
        Observation, ObservationEvents, ObservationRegistration, ObservationRequest,
        ObservationSession, WindowObservationEvent, WindowObservationNotification,
        WindowObservationStart,
    },
    selection::{TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, PlatformError, PlatformFuture,
};
use serde::Deserialize;
use std::ffi::{c_char, c_void, CStr, CString};
use std::num::NonZeroU64;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

type PickerCallback = unsafe extern "C" fn(*const c_char, *mut c_void);
type WindowObservationCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

unsafe extern "C" {
    fn lens_accessibility_is_trusted() -> bool;
    fn lens_accessibility_request_trust() -> bool;
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

struct WindowObservationCallbackContext {
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    window_id: u32,
    sender: mpsc::Sender<WindowObservationEvent>,
}

struct NativeObservationRegistration {
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

unsafe extern "C" fn window_observation_callback(json: *const c_char, context: *mut c_void) {
    if json.is_null() || context.is_null() {
        return;
    }
    // SAFETY: The boxed context is owned by `NativeObservationRegistration` and remains at a
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

impl NativeObservationRegistration {
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

impl Drop for NativeObservationRegistration {
    fn drop(&mut self) {
        let _ = self.stop_inner();
        // If native reported an already-absent source, it cannot retain the callback context.
        self.callback_context = None;
    }
}

fn accessibility_is_trusted() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrusted.
    unsafe { lens_accessibility_is_trusted() }
}

fn request_accessibility_trust() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrustedWithOptions.
    unsafe { lens_accessibility_request_trust() }
}

async fn present_window_picker_for_operation(
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

fn extract_window(
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

fn extract_registered_window(
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

fn start_window_observation(
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    identity: &WindowIdentity,
) -> Result<
    (
        NativeObservationRegistration,
        mpsc::Receiver<WindowObservationEvent>,
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
    let registration = NativeObservationRegistration {
        operation_id,
        source_registration_id,
        callback_context: Some(callback_context),
    };
    Ok((registration, receiver, start))
}

fn release_registered_window(operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
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

fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
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

struct NativeObservationEvents(mpsc::Receiver<WindowObservationEvent>);

impl ObservationEvents for NativeObservationEvents {
    fn poll_next(
        &mut self,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<WindowObservationEvent>> {
        self.0.poll_recv(context)
    }
}

impl ObservationRegistration for NativeObservationRegistration {
    fn close(&mut self) -> Result<(), PlatformError> {
        self.stop_inner()
    }
}

impl TargetSelection for MacOsPlatform {
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        Box::pin(present_window_picker_for_operation(operation_id))
    }

    fn release_target(&self, operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
        release_registered_window(operation_id, window_id)
    }

    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError> {
        release_window_operation(operation_id)
    }
}

impl Accessibility for MacOsPlatform {
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError> {
        match target {
            ExtractionTarget::Registered {
                operation_id,
                identity,
            } => extract_registered_window(operation_id, &identity, limits),
            ExtractionTarget::Legacy(window) => extract_window(&window, limits),
        }
    }
}

impl Observation for MacOsPlatform {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        let (registration, receiver, start) = start_window_observation(
            request.operation_id,
            request.context_id,
            request.source_registration_id,
            request.observer_epoch,
            &request.identity,
        )?;
        Ok(ObservationSession {
            registration: Box::new(registration),
            events: Box::new(NativeObservationEvents(receiver)),
            start,
        })
    }
}

impl AccessibilityTrust for MacOsPlatform {
    fn inspect(&self) -> bool {
        accessibility_is_trusted()
    }

    fn request(&self) -> bool {
        request_accessibility_trust()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> WindowObservationEvent {
        WindowObservationEvent {
            operation_id: Uuid::from_u128(1),
            context_id: Uuid::from_u128(2),
            source_registration_id: Uuid::from_u128(3),
            observer_epoch: NonZeroU64::MIN,
            window_id: 17,
            notification: WindowObservationNotification::WindowTitleChanged,
        }
    }

    #[test]
    fn observer_callback_checks_every_authority_axis_and_coalesces_notifications() {
        let expected = event();
        let (sender, mut receiver) = mpsc::channel(1);
        let mut context = Box::new(WindowObservationCallbackContext {
            operation_id: expected.operation_id,
            context_id: expected.context_id,
            source_registration_id: expected.source_registration_id,
            observer_epoch: expected.observer_epoch,
            window_id: expected.window_id,
            sender,
        });
        let raw_context = (&mut *context as *mut WindowObservationCallbackContext).cast();
        for stale in [
            WindowObservationEvent {
                operation_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                context_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                source_registration_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                observer_epoch: NonZeroU64::new(2).unwrap(),
                ..expected.clone()
            },
            WindowObservationEvent {
                window_id: 18,
                ..expected.clone()
            },
        ] {
            let json = CString::new(serde_json::to_string(&stale).unwrap()).unwrap();
            // SAFETY: This test owns the callback context and JSON for the synchronous call.
            unsafe { window_observation_callback(json.as_ptr(), raw_context) };
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        let json = CString::new(serde_json::to_string(&expected).unwrap()).unwrap();
        // SAFETY: Owned buffers remain valid; observation callbacks only borrow the context.
        unsafe { window_observation_callback(json.as_ptr(), raw_context) };
        let mut later = expected.clone();
        later.notification = WindowObservationNotification::WindowResized;
        let second = CString::new(serde_json::to_string(&later).unwrap()).unwrap();
        // SAFETY: The same stable owned context is still alive for this second callback.
        unsafe { window_observation_callback(second.as_ptr(), raw_context) };
        assert_eq!(receiver.try_recv().unwrap(), expected);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let invalid = CString::new("invalid JSON").unwrap();
        // SAFETY: The invalid payload is still a valid C string; null input is explicitly handled.
        unsafe {
            window_observation_callback(invalid.as_ptr(), raw_context);
            window_observation_callback(std::ptr::null(), raw_context);
            window_observation_callback(json.as_ptr(), std::ptr::null_mut());
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn native_event_stream_distinguishes_pending_delivery_and_terminal_close() {
        let (sender, receiver) = mpsc::channel(1);
        let mut events = NativeObservationEvents(receiver);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(events.poll_next(&mut context).is_pending());
        sender.try_send(event()).unwrap();
        assert_eq!(
            events.poll_next(&mut context),
            std::task::Poll::Ready(Some(event()))
        );
        assert!(events.poll_next(&mut context).is_pending());
        drop(sender);
        assert_eq!(events.poll_next(&mut context), std::task::Poll::Ready(None));
        assert_eq!(events.poll_next(&mut context), std::task::Poll::Ready(None));
    }

    #[test]
    fn picker_callback_transfers_owned_terminal_response_once() {
        let (sender, mut receiver) = oneshot::channel();
        let raw_context = Box::into_raw(Box::new(PickerContext {
            sender: Some(sender),
        }))
        .cast();
        let response = CString::new(r#"{"status":"cancelled"}"#).unwrap();
        // SAFETY: Ownership of the allocated context transfers to this sole terminal callback.
        unsafe { picker_callback(response.as_ptr(), raw_context) };
        assert_eq!(receiver.try_recv().unwrap(), response.to_str().unwrap());

        let (sender, mut receiver) = oneshot::channel();
        let raw_context = Box::into_raw(Box::new(PickerContext {
            sender: Some(sender),
        }))
        .cast();
        // SAFETY: A new context transfers exactly once; a null native response is explicitly handled.
        unsafe { picker_callback(std::ptr::null(), raw_context) };
        let reply: WindowPickerReply = serde_json::from_str(&receiver.try_recv().unwrap()).unwrap();
        assert!(matches!(reply, WindowPickerReply::Error { .. }));

        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        let raw_context = Box::into_raw(Box::new(PickerContext {
            sender: Some(sender),
        }))
        .cast();
        // SAFETY: The callback still owns and releases its context after its receiver was cancelled.
        unsafe { picker_callback(response.as_ptr(), raw_context) };
    }

    #[test]
    fn observation_handle_and_receiver_can_move_to_the_live_source_actor() {
        fn assert_send<T: Send>() {}

        assert_send::<NativeObservationRegistration>();
        assert_send::<NativeObservationEvents>();
        assert_send::<port_platform::observation::ObservationSession>();
    }
}
