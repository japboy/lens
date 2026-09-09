use base64::prelude::*;
use port_platform::{
    capture::{
        Bounds, Capture, CaptureBatch, CaptureCoverage, CaptureOmission, CaptureOmissionReason,
        CaptureRequest, CaptureTarget, CapturedImage,
    },
    ImageCaptureLimits, PlatformError,
};
use serde::{Deserialize, Serialize};
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
    fn lens_capture_receipt_window_regions_json(
        operation_id: *const c_char,
        receipt: *const c_char,
        read_sequence: *const c_char,
        requests_json: *const c_char,
        max_long_edge: u32,
        max_pixels: u32,
        max_attachment_bytes: u32,
        max_total_bytes: u32,
    ) -> *mut c_char;
    fn lens_free_string(value: *mut c_char);
}

impl Capture for crate::MacOsPlatform {
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
        let expected = match target {
            CaptureTarget::Registered { read } => Some(read),
            CaptureTarget::Legacy { .. } => None,
        };
        super::validate_read(native.read, expected)?;
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
    read: Option<port_platform::authority::TargetReadKey>,
    geometry_observation: Option<crate::geometry::NativeGeometryObservation>,
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
    pixel_geometry: port_platform::geometry::CapturedPixelGeometry,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeCaptureScope {
    AxElementRegion,
    WindowFallback,
}

#[derive(Debug, Serialize)]
struct NativeCaptureRequest<'a> {
    id: &'a str,
    scope: NativeCaptureScope,
    bounds: Option<Bounds>,
}

impl<'a> From<&'a CaptureRequest> for NativeCaptureRequest<'a> {
    fn from(request: &'a CaptureRequest) -> Self {
        Self {
            id: &request.id,
            scope: match request.scope {
                port_platform::capture::CaptureScope::AccessibilityElementRegion => {
                    NativeCaptureScope::AxElementRegion
                }
                port_platform::capture::CaptureScope::WindowFallback => {
                    NativeCaptureScope::WindowFallback
                }
            },
            bounds: request.bounds,
        }
    }
}

fn capture_native(
    target: CaptureTarget,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<NativeImageCaptureBatch, PlatformError> {
    let requests = requests
        .iter()
        .map(NativeCaptureRequest::from)
        .collect::<Vec<_>>();
    let requests_json = serde_json::to_string(&requests).map_err(|error| {
        PlatformError::Operation(format!(
            "unable to serialize image capture requests: {error}"
        ))
    })?;
    let requests_json = CString::new(requests_json).map_err(|_| {
        PlatformError::Operation("image capture request JSON contains an interior NUL".into())
    })?;
    // SAFETY: All pointers remain valid for this blocking call. The registered path reads only
    // the exact operation-owned SCWindow; the legacy path is retained for explicit one-shot input.
    let raw = match target {
        CaptureTarget::Registered { read } => {
            let operation_id = CString::new(read.target.operation_id.to_string())
                .expect("UUID text contains no interior NUL");
            let receipt = CString::new(uuid::Uuid::from(read.target.receipt).to_string())
                .expect("UUID text contains no interior NUL");
            let sequence = CString::new(String::from(read.sequence)).expect("decimal has no NUL");
            unsafe {
                lens_capture_receipt_window_regions_json(
                    operation_id.as_ptr(),
                    receipt.as_ptr(),
                    sequence.as_ptr(),
                    requests_json.as_ptr(),
                    limits.max_long_edge,
                    limits.max_pixels,
                    limits.max_attachment_bytes,
                    limits.max_total_bytes,
                )
            }
        }
        CaptureTarget::Legacy { window_id } => unsafe {
            lens_capture_window_regions_json(
                window_id,
                requests_json.as_ptr(),
                limits.max_long_edge,
                limits.max_pixels,
                limits.max_attachment_bytes,
                limits.max_total_bytes,
            )
        },
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
    let geometry = match native.read {
        Some(_)
            if native.geometry_observation.is_none()
                && requests.iter().all(|request| {
                    request.scope == port_platform::capture::CaptureScope::WindowFallback
                }) =>
        {
            None
        }
        Some(read) if !native.captures.is_empty() => Some(crate::geometry::observed_descriptor(
            native.geometry_observation,
            read,
            native.window_bounds,
        )?),
        _ => None,
    };
    let mut result = CaptureBatch {
        capture: native
            .read
            .filter(|_| !native.captures.is_empty())
            .map(|read| port_platform::geometry::CaptureKey {
                read,
                capture_id: uuid::Uuid::new_v4(),
            }),
        read: native.read,
        geometry,
        window_bounds: native.window_bounds,
        diagnostics: native.diagnostics,
        ..CaptureBatch::default()
    };
    let mut resolved = BTreeSet::new();
    let mut total_bytes = 0_usize;
    for capture in native.captures {
        let request = requests_by_id
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
            || Some(capture.window_bounds) != result.window_bounds
            || match request.scope {
                port_platform::capture::CaptureScope::AccessibilityElementRegion => {
                    request.bounds != Some(capture.source_bounds)
                }
                port_platform::capture::CaptureScope::WindowFallback => {
                    capture.source_bounds != capture.window_bounds
                }
            }
            || capture.pixel_geometry.validate().is_err()
            || !valid_pixel_geometry(&capture)
            || usize::try_from(capture.pixel_geometry.encoded_extent.width).ok()
                != Some(capture.pixel_width)
            || usize::try_from(capture.pixel_geometry.encoded_extent.height).ok()
                != Some(capture.pixel_height)
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
            pixel_geometry: capture.pixel_geometry,
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

fn valid_pixel_geometry(capture: &NativeImageCapture) -> bool {
    let geometry = capture.pixel_geometry;
    // The native whole-window surface is bounded to 4096 pixels per edge; the
    // later attachment scaler only downsizes. These are adapter policy, not DPI.
    if geometry
        .original_extent
        .width
        .max(geometry.original_extent.height)
        > 4096
        || geometry.encoded_extent.width > geometry.crop.extent.width
        || geometry.encoded_extent.height > geometry.crop.extent.height
    {
        return false;
    }
    let window = capture.window_bounds;
    let region = capture.captured_bounds;
    let scale_x = f64::from(geometry.original_extent.width) / window.width;
    let scale_y = f64::from(geometry.original_extent.height) / window.height;
    let x = ((region.x - window.x) * scale_x).floor().max(0.0);
    let y = ((region.y - window.y) * scale_y).floor().max(0.0);
    let right = ((region.x + region.width - window.x) * scale_x)
        .ceil()
        .min(f64::from(geometry.original_extent.width));
    let bottom = ((region.y + region.height - window.y) * scale_y)
        .ceil()
        .min(f64::from(geometry.original_extent.height));
    [scale_x, scale_y, x, y, right, bottom]
        .iter()
        .all(|v| v.is_finite())
        && scale_x > 0.0
        && scale_y > 0.0
        && x == f64::from(geometry.crop.x)
        && y == f64::from(geometry.crop.y)
        && right - x == f64::from(geometry.crop.extent.width)
        && bottom - y == f64::from(geometry.crop.extent.height)
}

fn valid_capture_bounds(
    source: Bounds,
    captured: Bounds,
    coverage: CaptureCoverage,
    window: Bounds,
) -> bool {
    let valid = |bounds: Bounds| {
        [
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            bounds.x + bounds.width,
            bounds.y + bounds.height,
        ]
        .into_iter()
        .all(f64::is_finite)
            && bounds.width > 0.0
            && bounds.height > 0.0
            && bounds.x + bounds.width > bounds.x
            && bounds.y + bounds.height > bounds.y
    };
    if !valid(source) || !valid(captured) || !valid(window) {
        return false;
    }
    // Mirror the native rectangle construction, not reconstructed edges: adding
    // a large negative origin to a width can lose many ULPs near zero.
    let expected = [
        source.x.max(window.x),
        source.y.max(window.y),
        (source.x + source.width).min(window.x + window.width),
        (source.y + source.height).min(window.y + window.height),
    ];
    let source_edges = [
        source.x,
        source.y,
        source.x + source.width,
        source.y + source.height,
    ];
    let same_region = expected == source_edges;
    match coverage {
        CaptureCoverage::FullRegion => same_region && captured == source,
        CaptureCoverage::VisibleSubregion => {
            !same_region
                && captured.x == expected[0]
                && captured.y == expected[1]
                && captured.width == expected[2] - expected[0]
                && captured.height == expected[3] - expected[1]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_scopes_map_to_exact_private_native_wire() {
        use port_platform::capture::CaptureScope;
        for (scope, public, native) in [
            (
                CaptureScope::AccessibilityElementRegion,
                "accessibility_element_region",
                "ax_element_region",
            ),
            (
                CaptureScope::WindowFallback,
                "window_fallback",
                "window_fallback",
            ),
        ] {
            for bounds in [
                None,
                Some(Bounds {
                    x: -1.5,
                    y: 2.0,
                    width: 3.5,
                    height: 4.0,
                }),
            ] {
                let request = CaptureRequest {
                    id: "region-1".into(),
                    scope,
                    bounds,
                };
                assert_eq!(serde_json::to_value(&request).unwrap()["scope"], public);
                assert_eq!(
                    serde_json::to_value(NativeCaptureRequest::from(&request)).unwrap(),
                    serde_json::json!({"id":"region-1", "scope":native, "bounds":bounds})
                );
            }
        }
    }

    fn requests() -> Vec<CaptureRequest> {
        vec![CaptureRequest {
            id: "media-1".into(),
            scope: port_platform::capture::CaptureScope::AccessibilityElementRegion,
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
                "pixel_geometry": {
                    "original_extent":{"width":100,"height":100},
                    "crop":{"x":1,"y":2,"extent":{"width":3,"height":4}},
                    "encoded_extent":{"width":3,"height":4}
                },
                "source_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "captured_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "window_bounds":{"x":0.0,"y":0.0,"width":100.0,"height":100.0},
                "coverage":"full_region", "mime_type":"image/png",
                "pixel_width":3, "pixel_height":4, "encoded_bytes":3, "data":"AQID"
            }],
            "diagnostics":["native diagnostic"]
        })
    }

    #[test]
    fn registered_region_capture_requires_stable_observed_geometry() {
        let mut wire = native_fixture();
        wire["read"] = serde_json::json!({
            "target":{"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000002"},
            "sequence":"1"
        });
        assert!(validate_fixture(wire.clone()).is_err());
        wire["geometry_observation"] = serde_json::json!({
            "before":wire["window_bounds"], "after":wire["window_bounds"]
        });
        let result = validate_fixture(wire.clone()).unwrap();
        let capture = result.capture.unwrap();
        assert_eq!(Some(capture.read), result.read);
        assert!(!capture.capture_id.is_nil());
        assert_eq!(result.geometry.as_ref().unwrap().read, result.read.unwrap());
        wire["geometry_observation"]["after"]["x"] = serde_json::json!(1.0);
        assert!(validate_fixture(wire.clone()).is_err());
        wire["geometry_observation"]["before"]["x"] = serde_json::json!(1.0);
        assert!(validate_fixture(wire).is_err());
    }

    #[test]
    fn whole_window_without_ax_keeps_desktop_mapping_unknown() {
        let mut wire = native_fixture();
        wire["read"] = serde_json::json!({
            "target":{"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000002"},
            "sequence":"1"
        });
        let window = wire["window_bounds"].clone();
        wire["captures"][0]["source_bounds"] = window.clone();
        wire["captures"][0]["captured_bounds"] = window;
        wire["captures"][0]["pixel_geometry"]["crop"] = serde_json::json!({
            "x":0,"y":0,"extent":{"width":100,"height":100}
        });
        let requests = [CaptureRequest {
            id: "media-1".into(),
            scope: port_platform::capture::CaptureScope::WindowFallback,
            bounds: None,
        }];
        let result = validate_native(
            serde_json::from_value(wire.clone()).unwrap(),
            &requests,
            limits(),
        )
        .unwrap();
        assert!(result.geometry.is_none());
        assert_eq!(result.captures.len(), 1);
        wire["geometry_observation"] = serde_json::json!({
            "before":wire["window_bounds"], "after":wire["window_bounds"]
        });
        wire["geometry_observation"]["after"]["x"] = serde_json::json!(1.0);
        assert!(
            validate_native(serde_json::from_value(wire).unwrap(), &requests, limits()).is_err()
        );
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
                pixel_geometry: serde_json::from_value(
                    native_fixture()["captures"][0]["pixel_geometry"].clone()
                )
                .unwrap(),
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
    fn pixel_provenance_is_required_and_validated_before_admission() {
        let mut missing = native_fixture();
        missing["captures"][0]
            .as_object_mut()
            .unwrap()
            .remove("pixel_geometry");
        assert!(serde_json::from_value::<NativeImageCaptureBatch>(missing).is_err());
        for (field, value) in [
            (
                "original_extent",
                serde_json::json!({"width":0,"height":100}),
            ),
            (
                "crop",
                serde_json::json!({"x":99,"y":2,"extent":{"width":3,"height":4}}),
            ),
            (
                "crop",
                serde_json::json!({"x":2,"y":2,"extent":{"width":3,"height":4}}),
            ),
            (
                "crop",
                serde_json::json!({"x":4294967295_u32,"y":2,"extent":{"width":3,"height":4}}),
            ),
            ("encoded_extent", serde_json::json!({"width":2,"height":4})),
        ] {
            let mut invalid = native_fixture();
            invalid["captures"][0]["pixel_geometry"][field] = value;
            assert!(validate_fixture(invalid).is_err(), "{field}");
        }
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
            // Each case changes one declaration while retaining the same request read.
            let mut fixture = native_fixture();
            fixture["captures"][0][field] = value;
            assert!(
                validate_fixture(fixture).is_err(),
                "accepted invalid {field}"
            );
        }
    }

    #[test]
    fn fractional_region_keeps_integer_crop_and_resized_extent_separate() {
        let mut fixture = native_fixture();
        let window = bounds(-10.25, -20.5, 100.0, 100.0);
        let source = bounds(-9.95, -20.2, 1.1, 0.8);
        fixture["window_bounds"] = serde_json::to_value(window).unwrap();
        let capture = &mut fixture["captures"][0];
        capture["window_bounds"] = serde_json::to_value(window).unwrap();
        capture["source_bounds"] = serde_json::to_value(source).unwrap();
        capture["captured_bounds"] = serde_json::to_value(source).unwrap();
        capture["pixel_width"] = serde_json::json!(2);
        capture["pixel_height"] = serde_json::json!(2);
        capture["pixel_geometry"] = serde_json::json!({
            "original_extent":{"width":200,"height":300},
            "crop":{"x":0,"y":0,"extent":{"width":3,"height":4}},
            "encoded_extent":{"width":2,"height":2}
        });
        let mut requests = requests();
        requests[0].bounds = Some(source);
        let result = validate_native(
            serde_json::from_value(fixture.clone()).unwrap(),
            &requests,
            limits(),
        )
        .unwrap();
        let image = &result.captures[0];
        assert_eq!(image.coverage, CaptureCoverage::FullRegion);
        assert_eq!(image.pixel_geometry.crop.extent.width, 3);
        assert_eq!(image.pixel_geometry.encoded_extent.width, 2);
        assert_eq!(image.captured_bounds, source);
        requests[0].bounds = Some(window);
        assert!(validate_native(
            serde_json::from_value(fixture.clone()).unwrap(),
            &requests,
            limits()
        )
        .is_err());
        fixture["window_bounds"]["x"] = serde_json::json!(0.0);
        requests[0].bounds = Some(source);
        assert!(validate_native(
            serde_json::from_value(fixture).unwrap(),
            &requests,
            limits()
        )
        .is_err());
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
        assert!(!valid_capture_bounds(
            window,
            bounds(0.0, 0.0, 10.0, 10.0),
            CaptureCoverage::VisibleSubregion,
            window
        ));
        assert!(valid_capture_bounds(
            bounds(-0.1, 0.0, 1.0, 1.0),
            bounds(-0.1, 0.0, 0.2 - (-0.1), 1.0),
            CaptureCoverage::VisibleSubregion,
            bounds(-0.2, 0.0, 0.4, 1.0),
        ));
        let source = bounds(-1000.1, 0.0, 2000.0, 1.0);
        let window = bounds(-2000.0, 0.0, 2000.2, 1.0);
        let width = (window.x + window.width) - source.x;
        assert!(valid_capture_bounds(
            source,
            bounds(source.x, 0.0, width, 1.0),
            CaptureCoverage::VisibleSubregion,
            window,
        ));
        assert!(!valid_capture_bounds(
            source,
            bounds(source.x, 0.0, width.next_down(), 1.0),
            CaptureCoverage::VisibleSubregion,
            window,
        ));
    }
}
