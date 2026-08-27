use super::PlatformError;
use crate::model::{ExtractionResult, SelectedWindow, WindowPickerReply};
use std::ffi::{c_char, c_void, CStr, CString};
use tokio::sync::oneshot;

type PickerCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

unsafe extern "C" {
    fn pl_accessibility_is_trusted() -> bool;
    fn pl_accessibility_request_trust() -> bool;
    fn pl_present_window_picker(callback: PickerCallback, context: *mut c_void) -> bool;
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
