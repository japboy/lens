use base64::prelude::*;
use port_platform::{
    capture::{
        Bounds, Capture, CaptureBatch, CaptureCoverage, CaptureOmission, CaptureOmissionReason,
        CaptureRequest, CaptureTarget, CapturedImage,
    },
    ImageCaptureLimits, PlatformError,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, CStr, CString};

unsafe extern "C" {
    fn lens_capture_window_regions_json(
        window_id: u32,
        requests_json: *const c_char,
        max_long_edge: u32,
        max_pixels: u32,
        max_attachment_bytes: u32,
        max_total_bytes: u32,
    ) -> *mut c_char;
    fn lens_capture_registered_window_regions_json(
        operation_id: *const c_char,
        window_id: u32,
        requests_json: *const c_char,
        max_long_edge: u32,
        max_pixels: u32,
        max_attachment_bytes: u32,
        max_total_bytes: u32,
    ) -> *mut c_char;
    fn lens_free_string(value: *mut c_char);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsCapture;

impl Capture for MacOsCapture {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        limits: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        if requests.is_empty() {
            return Ok(CaptureBatch::default());
        }
        request_index(requests)?;
        let native = capture_native(target, requests, limits)?;
        validate_native(native, requests, limits)
    }
}

fn request_index(
    requests: &[CaptureRequest],
) -> Result<BTreeMap<&str, &CaptureRequest>, PlatformError> {
    let index = requests
        .iter()
        .map(|request| (request.id.as_str(), request))
        .collect::<BTreeMap<_, _>>();
    if index.len() != requests.len() {
        return Err(PlatformError::Operation(
            "image capture request identities must be unique".into(),
        ));
    }
    Ok(index)
}

#[derive(Debug, Deserialize)]
struct NativeImageCaptureBatch {
    #[serde(default)]
    window_bounds: Option<Bounds>,
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
    coverage: CaptureCoverage,
    mime_type: String,
    pixel_width: usize,
    pixel_height: usize,
    encoded_bytes: usize,
    data: String,
}

#[derive(Debug, Deserialize)]
struct NativeImageOmission {
    attachment_id: String,
    reason: CaptureOmissionReason,
    detail: String,
}

fn capture_native(
    target: CaptureTarget,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<NativeImageCaptureBatch, PlatformError> {
    let requests_json = serde_json::to_string(requests).map_err(|error| {
        PlatformError::Operation(format!(
            "unable to serialize image capture requests: {error}"
        ))
    })?;
    let requests_json = CString::new(requests_json).map_err(|_| {
        PlatformError::Operation("image capture request JSON contains an interior NUL".into())
    })?;
    let (operation_id, window_id) = match target {
        CaptureTarget::Registered {
            operation_id,
            window_id,
        } => (Some(operation_id), window_id),
        CaptureTarget::Legacy { window_id } => (None, window_id),
    };
    let operation_id = operation_id
        .map(|id| CString::new(id.to_string()).expect("UUID text contains no interior NUL"));
    // SAFETY: All pointers remain valid for this blocking call. The registered path reads only
    // the exact operation-owned SCWindow; the legacy path is retained for explicit one-shot input.
    let raw = if let Some(operation_id) = operation_id.as_ref() {
        unsafe {
            lens_capture_registered_window_regions_json(
                operation_id.as_ptr(),
                window_id,
                requests_json.as_ptr(),
                limits.max_long_edge,
                limits.max_pixels,
                limits.max_attachment_bytes,
                limits.max_total_bytes,
            )
        }
    } else {
        unsafe {
            lens_capture_window_regions_json(
                window_id,
                requests_json.as_ptr(),
                limits.max_long_edge,
                limits.max_pixels,
                limits.max_attachment_bytes,
                limits.max_total_bytes,
            )
        }
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

    Ok(native)
}

fn validate_native(
    native: NativeImageCaptureBatch,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<CaptureBatch, PlatformError> {
    let requests_by_id = request_index(requests)?;
    let mut result = CaptureBatch {
        window_bounds: native.window_bounds,
        diagnostics: native.diagnostics,
        ..CaptureBatch::default()
    };
    let mut resolved = BTreeSet::new();
    let mut total_bytes = 0_usize;
    for capture in native.captures {
        let _request = requests_by_id
            .get(capture.attachment_id.as_str())
            .ok_or_else(|| {
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
        result.captures.push(CapturedImage {
            attachment_id: capture.attachment_id,
            source_bounds: capture.source_bounds,
            captured_bounds: capture.captured_bounds,
            coverage: capture.coverage,
            pixel_width: capture.pixel_width,
            pixel_height: capture.pixel_height,
            png: decoded,
        });
    }
    for omission in native.omissions {
        let _request = requests_by_id
            .get(omission.attachment_id.as_str())
            .ok_or_else(|| {
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
        result.omissions.push(CaptureOmission {
            attachment_id: omission.attachment_id,
            reason: omission.reason,
            detail: omission.detail,
        });
    }
    Ok(result)
}

fn valid_capture_bounds(
    source: Bounds,
    captured: Bounds,
    coverage: CaptureCoverage,
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
            CaptureCoverage::FullRegion => same_region,
            CaptureCoverage::VisibleSubregion => !same_region,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requests() -> Vec<CaptureRequest> {
        vec![CaptureRequest {
            id: "media-1".into(),
            scope: port_platform::capture::CaptureScope::AxElementRegion,
            bounds: Some(bounds(1.0, 2.0, 3.0, 4.0)),
        }]
    }

    fn limits() -> ImageCaptureLimits {
        ImageCaptureLimits {
            max_long_edge: 10,
            max_pixels: 100,
            max_attachment_bytes: 8,
            max_total_bytes: 8,
        }
    }

    fn native_fixture() -> serde_json::Value {
        // Transfer validation fixture, not a PNG decoder or native pixel probe.
        serde_json::json!({
            "window_bounds": {"x":0.0,"y":0.0,"width":100.0,"height":100.0},
            "captures":[{
                "attachment_id":"media-1",
                "source_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "captured_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "window_bounds":{"x":0.0,"y":0.0,"width":100.0,"height":100.0},
                "coverage":"full_region", "mime_type":"image/png",
                "pixel_width":3, "pixel_height":4, "encoded_bytes":3, "data":"AQID"
            }],
            "diagnostics":["native diagnostic"]
        })
    }

    fn validate_fixture(value: serde_json::Value) -> Result<CaptureBatch, PlatformError> {
        validate_native(
            serde_json::from_value(value).unwrap(),
            &requests(),
            limits(),
        )
    }

    #[test]
    fn native_transfer_decodes_to_owned_bytes_and_current_facts() {
        let result = validate_fixture(native_fixture()).unwrap();
        assert_eq!(
            result.captures,
            vec![CapturedImage {
                attachment_id: "media-1".into(),
                source_bounds: bounds(1.0, 2.0, 3.0, 4.0),
                captured_bounds: bounds(1.0, 2.0, 3.0, 4.0),
                coverage: CaptureCoverage::FullRegion,
                pixel_width: 3,
                pixel_height: 4,
                png: vec![1, 2, 3],
            }]
        );
        assert_eq!(result.window_bounds, Some(bounds(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(result.diagnostics, ["native diagnostic"]);
        assert!(result.omissions.is_empty());
    }

    #[test]
    fn malformed_native_transfer_is_rejected() {
        for (field, value) in [
            ("data", serde_json::json!("not base64!")),
            ("mime_type", serde_json::json!("image/jpeg")),
            ("encoded_bytes", serde_json::json!(4)),
            ("pixel_width", serde_json::json!(0)),
            ("pixel_height", serde_json::json!(0)),
            ("pixel_width", serde_json::json!(11)),
            ("coverage", serde_json::json!("visible_subregion")),
            (
                "captured_bounds",
                serde_json::json!({"x":99.0,"y":99.0,"width":3.0,"height":4.0}),
            ),
        ] {
            // Each case changes one declaration while retaining the same request authority.
            let mut fixture = native_fixture();
            fixture["captures"][0][field] = value;
            assert!(
                validate_fixture(fixture).is_err(),
                "accepted invalid {field}"
            );
        }
    }

    #[test]
    fn request_and_terminal_identity_collisions_are_rejected() {
        let mut duplicate_requests = requests();
        duplicate_requests.push(duplicate_requests[0].clone());
        assert!(request_index(&duplicate_requests).is_err());
        let mut unknown = native_fixture();
        unknown["captures"][0]["attachment_id"] = serde_json::json!("unknown");
        let mut duplicate = native_fixture();
        let first = duplicate["captures"][0].clone();
        duplicate["captures"].as_array_mut().unwrap().push(first);
        let mut conflict = native_fixture();
        conflict["omissions"] = serde_json::json!([{"attachment_id":"media-1","reason":"capture_failed","detail":"conflict"}]);
        let mut unknown_omission = native_fixture();
        unknown_omission["omissions"] = serde_json::json!([{"attachment_id":"unknown","reason":"capture_failed","detail":"unknown"}]);
        for fixture in [unknown, duplicate, conflict, unknown_omission] {
            assert!(matches!(
                validate_fixture(fixture),
                Err(PlatformError::InvalidResponse(_))
            ));
        }
    }

    #[test]
    fn pixel_attachment_and_batch_budgets_remain_independent() {
        for limits in [
            ImageCaptureLimits {
                max_pixels: 11,
                ..limits()
            },
            ImageCaptureLimits {
                max_attachment_bytes: 2,
                ..limits()
            },
            ImageCaptureLimits {
                max_total_bytes: 2,
                ..limits()
            },
        ] {
            assert!(validate_native(
                serde_json::from_value(native_fixture()).unwrap(),
                &requests(),
                limits
            )
            .is_err());
        }
    }

    #[test]
    fn explicit_native_omission_is_not_a_synthetic_capture() {
        let fixture = serde_json::json!({"omissions":[{
            "attachment_id":"media-1", "reason":"outside_window", "detail":"outside"
        }]});
        let result = validate_fixture(fixture).unwrap();
        assert!(result.captures.is_empty());
        assert_eq!(
            result.omissions,
            [CaptureOmission {
                attachment_id: "media-1".into(),
                reason: CaptureOmissionReason::OutsideWindow,
                detail: "outside".into(),
            }]
        );
    }

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
            CaptureCoverage::FullRegion,
            window,
        ));

        let source = bounds(-10.0, 20.0, 40.0, 40.0);
        let visible = bounds(0.0, 20.0, 30.0, 40.0);
        assert!(valid_capture_bounds(
            source,
            visible,
            CaptureCoverage::VisibleSubregion,
            window,
        ));
        assert!(!valid_capture_bounds(
            source,
            visible,
            CaptureCoverage::FullRegion,
            window,
        ));
        assert!(!valid_capture_bounds(
            full,
            full,
            CaptureCoverage::VisibleSubregion,
            window,
        ));
    }
}
