#![forbid(unsafe_code)]

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
        let native = match capture_native(target, requests, limits) {
            Ok(native) => native,
            // Preserve the capability's existing unavailable-capture shape.
            // Malformed requests and invalid returned metadata remain errors.
            Err(PlatformError::Operation(message)) => {
                return Ok(CaptureBatch {
                    diagnostics: vec![message],
                    ..CaptureBatch::default()
                });
            }
            Err(error) => return Err(error),
        };
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
    use image::ImageEncoder;
    use screencapturekit::{
        async_api::{AsyncSCScreenshotManager, AsyncSCShareableContent},
        screenshot_manager::CGImageExt,
        stream::{configuration::SCStreamConfiguration, content_filter::SCContentFilter},
    };
    if [
        limits.max_long_edge,
        limits.max_pixels,
        limits.max_attachment_bytes,
        limits.max_total_bytes,
    ]
    .contains(&0)
    {
        return Err(PlatformError::Operation(
            "Image capture limits must be greater than zero.".into(),
        ));
    }
    request_index(requests)?;
    if requests.is_empty() {
        return Ok(NativeImageCaptureBatch {
            window_bounds: None,
            captures: Vec::new(),
            omissions: Vec::new(),
            diagnostics: Vec::new(),
        });
    }
    let (window, window_bounds) = match target {
        CaptureTarget::Registered {
            operation_id,
            window_id,
        } => crate::source::current_capture_source(operation_id, window_id)?,
        CaptureTarget::Legacy { window_id } => {
            let content = wait_for_capture(
                AsyncSCShareableContent::get(),
                std::time::Duration::from_secs(15),
            )?
            .map_err(capture_error)?;
            let window = content
                .windows()
                .into_iter()
                .find(|window| window.window_id() == window_id)
                .ok_or_else(|| capture_error("The explicit selected window is unavailable."))?;
            let frame = window.frame();
            let bounds = Bounds {
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.size.width,
                height: frame.size.height,
            };
            (window, bounds)
        }
    };
    if !finite_bounds(window_bounds) {
        return Err(capture_error(
            "The exact selected window did not expose finite current capture geometry.",
        ));
    }
    let filter = SCContentFilter::create()
        .with_window(&window)
        .build()
        .map_err(capture_error)?;
    let scale = 2.0_f64
        .min(4096.0 / window_bounds.width)
        .min(4096.0 / window_bounds.height);
    let configuration = SCStreamConfiguration::new()
        .with_width((window_bounds.width * scale).floor().max(1.0) as u32)
        .with_height((window_bounds.height * scale).floor().max(1.0) as u32)
        .with_scales_to_fit(true)
        .with_preserves_aspect_ratio(true)
        .map_err(capture_error)?
        .with_shows_cursor(false)
        .with_ignores_shadows_single_window(true)
        .map_err(capture_error)?
        .with_should_be_opaque(true)
        .map_err(capture_error)?;
    let image = wait_for_capture(
        AsyncSCScreenshotManager::capture_image(&filter, &configuration),
        std::time::Duration::from_secs(15),
    )?
    .map_err(capture_error)?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(capture_error(
            "ScreenCaptureKit returned invalid or unbounded pixel dimensions.",
        ));
    }
    // The dependency renders tightly packed premultiplied RGBA, independent of the
    // CGImage's native channel order and padded row stride. PNG stores straight alpha.
    let mut pixels = image.rgba_data().map_err(capture_error)?;
    unpremultiply_rgba(&mut pixels);
    let bitmap = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
        .ok_or_else(|| capture_error("ScreenCaptureKit returned an invalid RGBA buffer."))?;
    let mut batch = NativeImageCaptureBatch {
        window_bounds: Some(window_bounds),
        captures: Vec::new(),
        omissions: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut total_bytes = 0usize;
    for request in requests {
        let source_bounds = match request.scope {
            port_platform::capture::CaptureScope::WindowFallback => window_bounds,
            port_platform::capture::CaptureScope::AxElementRegion => match request.bounds {
                Some(bounds) if finite_bounds(bounds) => bounds,
                Some(_) => {
                    omit(
                        &mut batch,
                        request,
                        CaptureOmissionReason::InvalidBounds,
                        "AX image-region request has invalid bounds.",
                    );
                    continue;
                }
                None => {
                    omit(
                        &mut batch,
                        request,
                        CaptureOmissionReason::MissingBounds,
                        "AX image-region request has no bounds.",
                    );
                    continue;
                }
            },
        };
        let Some(captured_bounds) = intersection(window_bounds, source_bounds) else {
            omit(
                &mut batch,
                request,
                CaptureOmissionReason::OutsideWindow,
                "AX image region does not intersect the current selected-window frame.",
            );
            continue;
        };
        let Some((x, y, crop_width, crop_height)) =
            pixel_region(window_bounds, captured_bounds, width as u32, height as u32)
        else {
            omit(
                &mut batch,
                request,
                CaptureOmissionReason::OutsideWindow,
                "AX image region produced an empty pixel intersection.",
            );
            continue;
        };
        let cropped = image::imageops::crop_imm(&bitmap, x, y, crop_width, crop_height).to_image();
        let (target_width, target_height) = bounded_dimensions(crop_width, crop_height, limits);
        let bounded = if (target_width, target_height) == (crop_width, crop_height) {
            cropped
        } else {
            image::imageops::resize(
                &cropped,
                target_width,
                target_height,
                image::imageops::FilterType::Lanczos3,
            )
        };
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                bounded.as_raw(),
                target_width,
                target_height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(capture_error)?;
        if png.len() > limits.max_attachment_bytes as usize
            || png.len() > (limits.max_total_bytes as usize).saturating_sub(total_bytes)
        {
            omit(
                &mut batch,
                request,
                CaptureOmissionReason::ByteBudget,
                "Encoded PNG exceeded the versioned per-attachment or total media byte budget.",
            );
            continue;
        }
        total_bytes += png.len();
        batch.captures.push(NativeImageCapture {
            attachment_id: request.id.clone(),
            source_bounds,
            captured_bounds,
            window_bounds,
            coverage: if valid_capture_bounds(
                source_bounds,
                captured_bounds,
                CaptureCoverage::FullRegion,
                window_bounds,
            ) {
                CaptureCoverage::FullRegion
            } else {
                CaptureCoverage::VisibleSubregion
            },
            mime_type: "image/png".into(),
            pixel_width: target_width as usize,
            pixel_height: target_height as usize,
            encoded_bytes: png.len(),
            data: BASE64_STANDARD.encode(png),
        });
    }
    Ok(batch)
}

fn capture_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Operation(error.to_string())
}

fn finite_bounds(bounds: Bounds) -> bool {
    [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f64::is_finite)
        && bounds.width > 0.0
        && bounds.height > 0.0
        && (bounds.x + bounds.width).is_finite()
        && (bounds.y + bounds.height).is_finite()
}

fn intersection(left: Bounds, right: Bounds) -> Option<Bounds> {
    if !finite_bounds(left) || !finite_bounds(right) {
        return None;
    }
    // Preserve an entirely contained request verbatim. Reconstructing width by
    // subtracting endpoints introduces rounding that would mislabel full coverage.
    if right.x >= left.x
        && right.y >= left.y
        && right.x + right.width <= left.x + left.width
        && right.y + right.height <= left.y + left.height
    {
        return Some(right);
    }
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let width = (left.x + left.width).min(right.x + right.width) - x;
    let height = (left.y + left.height).min(right.y + right.height) - y;
    (width > 0.0 && height > 0.0).then_some(Bounds {
        x,
        y,
        width,
        height,
    })
}

fn pixel_region(
    window: Bounds,
    captured: Bounds,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let scale_x = f64::from(width) / window.width;
    let scale_y = f64::from(height) / window.height;
    let x = ((captured.x - window.x) * scale_x)
        .floor()
        .max(0.0)
        .min(f64::from(width)) as u32;
    let y = ((captured.y - window.y) * scale_y)
        .floor()
        .max(0.0)
        .min(f64::from(height)) as u32;
    let max_x = ((captured.x + captured.width - window.x) * scale_x)
        .ceil()
        .max(0.0)
        .min(f64::from(width)) as u32;
    let max_y = ((captured.y + captured.height - window.y) * scale_y)
        .ceil()
        .max(0.0)
        .min(f64::from(height)) as u32;
    (max_x > x && max_y > y).then_some((x, y, max_x.saturating_sub(x), max_y.saturating_sub(y)))
}

fn bounded_dimensions(width: u32, height: u32, limits: ImageCaptureLimits) -> (u32, u32) {
    let scale = 1.0_f64
        .min(f64::from(limits.max_long_edge) / f64::from(width.max(height)))
        .min((f64::from(limits.max_pixels) / (f64::from(width) * f64::from(height))).sqrt());
    let mut width = (f64::from(width) * scale).floor().max(1.0) as u32;
    let mut height = (f64::from(height) * scale).floor().max(1.0) as u32;
    // Clamping the short side to one pixel must not make an extreme aspect ratio
    // exceed the independent pixel budget.
    if u64::from(width) * u64::from(height) > u64::from(limits.max_pixels) {
        if width >= height {
            width = (limits.max_pixels / height).max(1);
        } else {
            height = (limits.max_pixels / width).max(1);
        }
    }
    (width, height)
}

fn unpremultiply_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
}

fn omit(
    batch: &mut NativeImageCaptureBatch,
    request: &CaptureRequest,
    reason: CaptureOmissionReason,
    detail: &str,
) {
    batch.omissions.push(NativeImageOmission {
        attachment_id: request.id.clone(),
        reason,
        detail: detail.into(),
    });
}

/// Poll an owned completion until its finite deadline, without assuming a Tokio
/// runtime or leaking a permanently waiting worker thread after timeout.
fn wait_for_capture<F: std::future::Future>(
    future: F,
    timeout: std::time::Duration,
) -> Result<F::Output, PlatformError> {
    use std::{
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        time::Instant,
    };
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + timeout;
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return Ok(result);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(capture_error(
                "ScreenCaptureKit image-region capture timed out after 15 seconds.",
            ));
        };
        std::thread::park_timeout(remaining);
    }
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
    fn public_capture_preserves_unavailable_batch_and_rejects_bad_request_authority() {
        let target = CaptureTarget::Legacy { window_id: 1 };
        let invalid_limits = ImageCaptureLimits {
            max_long_edge: 0,
            ..limits()
        };
        let batch = crate::MacOsPlatform
            .capture(target, &requests(), invalid_limits)
            .unwrap();
        assert!(batch.captures.is_empty() && batch.omissions.is_empty());
        assert_eq!(batch.diagnostics.len(), 1);
        let mut duplicate = requests();
        duplicate.push(duplicate[0].clone());
        assert!(crate::MacOsPlatform
            .capture(target, &duplicate, limits())
            .is_err());
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

    #[test]
    fn crop_rounding_preserves_fractional_visible_region_on_negative_desktops() {
        let window = bounds(-100.0, -50.0, 100.0, 80.0);
        let requested = bounds(-110.0, -49.25, 30.25, 20.5);
        let captured = intersection(window, requested).unwrap();
        assert_eq!(captured, bounds(-100.0, -49.25, 20.25, 20.5));
        assert_eq!(
            pixel_region(window, captured, 200, 160),
            Some((0, 1, 41, 42))
        );
        assert!(intersection(window, bounds(0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(intersection(window, bounds(f64::NAN, 0.0, 1.0, 1.0)).is_none());
    }

    #[test]
    fn fully_contained_fractional_region_preserves_exact_full_coverage() {
        let window = bounds(0.0, 0.0, 10.0, 10.0);
        let source = bounds(0.1, 0.1, 0.2, 0.2);
        let captured = intersection(window, source).unwrap();
        assert_eq!(captured, source);
        assert!(valid_capture_bounds(
            source,
            captured,
            CaptureCoverage::FullRegion,
            window
        ));
    }

    #[test]
    fn sub_epsilon_clipping_uses_the_same_coverage_contract_as_validation() {
        let window = bounds(0.0, 0.0, 10.0, 10.0);
        let source = bounds(-0.0001, 0.0, 1.0, 1.0);
        let captured = intersection(window, source).unwrap();
        let coverage =
            if valid_capture_bounds(source, captured, CaptureCoverage::FullRegion, window) {
                CaptureCoverage::FullRegion
            } else {
                CaptureCoverage::VisibleSubregion
            };
        assert!(valid_capture_bounds(source, captured, coverage, window));
        assert_eq!(coverage, CaptureCoverage::FullRegion);
    }

    #[test]
    fn extreme_aspect_ratio_obeys_independent_pixel_and_edge_budgets() {
        let limits = ImageCaptureLimits {
            max_long_edge: 4096,
            max_pixels: 1,
            max_attachment_bytes: 1024,
            max_total_bytes: 1024,
        };
        assert_eq!(bounded_dimensions(4096, 1, limits), (1, 1));
        assert_eq!(bounded_dimensions(1, 4096, limits), (1, 1));
        let limits = ImageCaptureLimits {
            max_pixels: 100,
            max_long_edge: 16,
            ..limits
        };
        for (width, height) in [(4096, 1), (1, 4096), (256, 256), (7, 9)] {
            let (width, height) = bounded_dimensions(width, height, limits);
            assert!(width.max(height) <= 16);
            assert!(width * height <= 100);
        }
    }

    #[test]
    fn premultiplied_rgba_is_encoded_with_straight_png_alpha() {
        use image::ImageEncoder;
        let mut pixels = vec![64, 32, 0, 128, 200, 100, 50, 255, 0, 0, 0, 0];
        unpremultiply_rgba(&mut pixels);
        assert_eq!(pixels, [128, 64, 0, 128, 200, 100, 50, 255, 0, 0, 0, 0]);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, 3, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgba8().as_raw(),
            &pixels
        );
    }

    #[test]
    fn owned_capture_wait_handles_ready_wakeup_and_deadline_without_a_runtime() {
        use std::{
            future::Future,
            pin::Pin,
            sync::{Arc, Mutex},
            task::{Context, Poll, Waker},
            time::Duration,
        };
        assert_eq!(
            wait_for_capture(std::future::ready(7), Duration::ZERO).unwrap(),
            7
        );
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
        let state = Arc::new(Mutex::new(State {
            ready: false,
            waker: None,
        }));
        let producer = Arc::clone(&state);
        let thread = std::thread::spawn(move || {
            // Wait until the consumer has installed its waker, then signal once.
            loop {
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
            }
        });
        assert_eq!(
            wait_for_capture(Completion(state), Duration::from_secs(2)).unwrap(),
            9
        );
        thread.join().unwrap();
        assert!(wait_for_capture(std::future::pending::<()>(), Duration::from_millis(1)).is_err());
    }
}
