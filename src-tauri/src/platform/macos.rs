use super::{ImageCaptureLimits, PlatformError};
use crate::{
    lens::{
        LensCoordinateSpace, LensMediaAttachment, LensMediaCapture, LensMediaCoverage,
        LensMediaOmission, LensMediaOmissionReason, LensMediaPayload, LensMediaPlan,
        MAX_AX_RESOURCE_REFERENCES, MAX_AX_RESOURCE_URI_BYTES, MAX_AX_TOTAL_RESOURCE_URI_BYTES,
    },
    model::{Bounds, ExtractionResult, SelectedWindow, WindowPickerReply},
};
use base64::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, c_void, CStr, CString};
use tokio::sync::oneshot;
use uuid::Uuid;

type PickerCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

unsafe extern "C" {
    fn lens_accessibility_is_trusted() -> bool;
    fn lens_accessibility_request_trust() -> bool;
    fn lens_present_window_picker(callback: PickerCallback, context: *mut c_void) -> bool;
    fn lens_capture_window_regions_json(
        window_id: u32,
        requests_json: *const c_char,
        max_long_edge: u32,
        max_pixels: u32,
        max_attachment_bytes: u32,
        max_total_bytes: u32,
    ) -> *mut c_char;
    fn lens_extract_window_json(
        pid: i32,
        selected_title: *const c_char,
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
    fn lens_free_string(value: *mut c_char);
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
    unsafe { lens_accessibility_is_trusted() }
}

pub fn request_accessibility_trust() -> bool {
    // SAFETY: This C function takes no pointers and delegates to AXIsProcessTrustedWithOptions.
    unsafe { lens_accessibility_request_trust() }
}

pub async fn present_window_picker() -> Result<WindowPickerReply, PlatformError> {
    let (sender, receiver) = oneshot::channel();
    let context = Box::new(PickerContext {
        sender: Some(sender),
    });
    let raw_context = Box::into_raw(context).cast::<c_void>();

    // SAFETY: The context remains owned by the callback when presentation succeeds. If
    // presentation fails synchronously, it is reconstructed below.
    let presented = unsafe { lens_present_window_picker(picker_callback, raw_context) };
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
        lens_extract_window_json(
            target.pid,
            title.as_ptr(),
            target.frame.x,
            target.frame.y,
            target.frame.width,
            target.frame.height,
            30_000,
            1_000_000,
            MAX_AX_RESOURCE_REFERENCES as u32,
            MAX_AX_RESOURCE_URI_BYTES as u32,
            MAX_AX_TOTAL_RESOURCE_URI_BYTES as u32,
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

#[derive(Debug, Deserialize)]
struct NativeImageCaptureBatch {
    #[serde(default)]
    captures: Vec<NativeImageCapture>,
    #[serde(default)]
    omissions: Vec<NativeImageOmission>,
    #[serde(default)]
    diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NativeImageCapture {
    attachment_id: String,
    source_bounds: Bounds,
    captured_bounds: Bounds,
    window_bounds: Bounds,
    coverage: LensMediaCoverage,
    mime_type: String,
    pixel_width: usize,
    pixel_height: usize,
    encoded_bytes: usize,
    data: String,
}

#[derive(Debug, Deserialize)]
struct NativeImageOmission {
    attachment_id: String,
    reason: LensMediaOmissionReason,
    detail: String,
}

pub fn capture_window_media(
    target: &SelectedWindow,
    context_id: Uuid,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    if plan.requests.is_empty() {
        return Ok(LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        });
    }
    let requests_by_id = plan
        .requests
        .iter()
        .map(|request| (request.id.clone(), request))
        .collect::<BTreeMap<_, _>>();
    if requests_by_id.len() != plan.requests.len() {
        return Err(PlatformError::Operation(
            "image capture request identities must be unique".into(),
        ));
    }
    let requests_json = serde_json::to_string(&plan.requests).map_err(|error| {
        PlatformError::Operation(format!(
            "unable to serialize image capture requests: {error}"
        ))
    })?;
    let requests_json = CString::new(requests_json).map_err(|_| {
        PlatformError::Operation("image capture request JSON contains an interior NUL".into())
    })?;
    // SAFETY: All pointer arguments remain valid for the duration of this blocking call. The
    // native bridge returns a malloc-owned NUL-terminated buffer released below.
    let raw = unsafe {
        lens_capture_window_regions_json(
            target.window_id,
            requests_json.as_ptr(),
            limits.max_long_edge,
            limits.max_pixels,
            limits.max_attachment_bytes,
            limits.max_total_bytes,
        )
    };
    if raw.is_null() {
        return Err(PlatformError::Operation(
            "native ScreenCaptureKit image-region capture returned no data".into(),
        ));
    }
    // SAFETY: `raw` is a valid NUL-terminated string returned by the native bridge.
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: The buffer was allocated by `PLCopyJSONString` and has not been freed yet.
    unsafe { lens_free_string(raw) };
    let native: NativeImageCaptureBatch = serde_json::from_str(&json).map_err(|error| {
        PlatformError::InvalidResponse(format!(
            "unable to decode native image capture response: {error}"
        ))
    })?;

    let mut result = LensMediaCapture {
        omissions: plan.omissions,
        diagnostics: native.diagnostics,
        ..LensMediaCapture::default()
    };
    let mut resolved = BTreeSet::new();
    let mut total_bytes = 0_usize;
    for capture in native.captures {
        let request = requests_by_id.get(&capture.attachment_id).ok_or_else(|| {
            PlatformError::InvalidResponse(format!(
                "native image capture returned unknown attachment {}",
                capture.attachment_id
            ))
        })?;
        if !resolved.insert(capture.attachment_id.clone()) {
            return Err(PlatformError::InvalidResponse(format!(
                "native image capture returned duplicate attachment {}",
                capture.attachment_id
            )));
        }
        let decoded = BASE64_STANDARD.decode(&capture.data).map_err(|error| {
            PlatformError::InvalidResponse(format!(
                "native image attachment {} is not valid base64: {error}",
                capture.attachment_id
            ))
        })?;
        if capture.mime_type != "image/png"
            || decoded.len() != capture.encoded_bytes
            || capture.pixel_width == 0
            || capture.pixel_height == 0
            || capture.pixel_width.max(capture.pixel_height) > limits.max_long_edge as usize
            || capture.pixel_width.saturating_mul(capture.pixel_height) > limits.max_pixels as usize
            || capture.encoded_bytes > limits.max_attachment_bytes as usize
            || !valid_capture_bounds(
                capture.source_bounds,
                capture.captured_bounds,
                capture.coverage,
                capture.window_bounds,
            )
        {
            return Err(PlatformError::InvalidResponse(format!(
                "native image attachment {} violates its declared format or finite limits",
                capture.attachment_id
            )));
        }
        total_bytes = total_bytes.saturating_add(capture.encoded_bytes);
        if total_bytes > limits.max_total_bytes as usize {
            return Err(PlatformError::InvalidResponse(
                "native image attachments exceed the total byte limit".into(),
            ));
        }
        let uri = format!(
            "lens://context/{context_id}/1/media/{}",
            capture.attachment_id
        );
        result.attachments.push(LensMediaAttachment {
            id: capture.attachment_id.clone(),
            uri: uri.clone(),
            scope: request.scope,
            source_node_id: request.source_node_id.clone(),
            source_bounds: capture.source_bounds,
            captured_bounds: capture.captured_bounds,
            coverage: capture.coverage,
            coordinate_space: LensCoordinateSpace::ScreenPoints,
            mime_type: capture.mime_type.clone(),
            pixel_width: capture.pixel_width,
            pixel_height: capture.pixel_height,
            encoded_bytes: capture.encoded_bytes,
        });
        result.payloads.push(LensMediaPayload {
            attachment_id: capture.attachment_id,
            uri,
            mime_type: capture.mime_type,
            data: capture.data,
        });
    }
    for omission in native.omissions {
        let request = requests_by_id.get(&omission.attachment_id).ok_or_else(|| {
            PlatformError::InvalidResponse(format!(
                "native image capture omitted unknown attachment {}",
                omission.attachment_id
            ))
        })?;
        if !resolved.insert(omission.attachment_id.clone()) {
            return Err(PlatformError::InvalidResponse(format!(
                "native image capture returned duplicate terminal outcome for attachment {}",
                omission.attachment_id
            )));
        }
        result.omissions.push(LensMediaOmission {
            attachment_id: Some(omission.attachment_id),
            source_node_id: request.source_node_id.clone(),
            reason: omission.reason,
            omitted_count: 1,
            first_order: None,
            last_order: None,
            detail: omission.detail,
        });
    }
    for request in &plan.requests {
        if !resolved.contains(&request.id) {
            result.omissions.push(LensMediaOmission {
                attachment_id: Some(request.id.clone()),
                source_node_id: request.source_node_id.clone(),
                reason: LensMediaOmissionReason::CaptureFailed,
                omitted_count: 1,
                first_order: None,
                last_order: None,
                detail: "Native image capture returned no terminal outcome for this request."
                    .into(),
            });
        }
    }
    Ok(result)
}

fn valid_capture_bounds(
    source: Bounds,
    captured: Bounds,
    coverage: LensMediaCoverage,
    window: Bounds,
) -> bool {
    let valid = |bounds: Bounds| {
        [bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f64::is_finite)
            && bounds.width > 0.0
            && bounds.height > 0.0
    };
    let contains = |outer: Bounds, inner: Bounds| {
        const EPSILON: f64 = 0.001;
        inner.x + EPSILON >= outer.x
            && inner.y + EPSILON >= outer.y
            && inner.x + inner.width <= outer.x + outer.width + EPSILON
            && inner.y + inner.height <= outer.y + outer.height + EPSILON
    };
    if !valid(source) || !valid(captured) || !valid(window) {
        return false;
    }
    let same_region = contains(source, captured) && contains(captured, source);
    contains(source, captured)
        && contains(window, captured)
        && match coverage {
            LensMediaCoverage::FullRegion => same_region,
            LensMediaCoverage::VisibleSubregion => !same_region,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn capture_coverage_must_match_declared_bounds() {
        let window = bounds(0.0, 0.0, 100.0, 100.0);
        let full = bounds(10.0, 20.0, 30.0, 40.0);
        assert!(valid_capture_bounds(
            full,
            full,
            LensMediaCoverage::FullRegion,
            window,
        ));

        let source = bounds(-10.0, 20.0, 40.0, 40.0);
        let visible = bounds(0.0, 20.0, 30.0, 40.0);
        assert!(valid_capture_bounds(
            source,
            visible,
            LensMediaCoverage::VisibleSubregion,
            window,
        ));
        assert!(!valid_capture_bounds(
            source,
            visible,
            LensMediaCoverage::FullRegion,
            window,
        ));
        assert!(!valid_capture_bounds(
            full,
            full,
            LensMediaCoverage::VisibleSubregion,
            window,
        ));
    }
}
