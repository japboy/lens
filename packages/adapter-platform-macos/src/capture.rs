#![forbid(unsafe_code)]

use image::ImageEncoder;
use port_platform::{
    capture::{
        Bounds, Capture, CaptureBatch, CaptureCoverage, CaptureOmission, CaptureOmissionReason,
        CaptureRegion, CaptureRequest, CaptureScope, CaptureTarget, CapturedImage,
    },
    geometry::{
        valid_bounds, AcquiredFrame, DesktopMapping, DesktopSpace, FrameId,
        GeometryUnavailableReason, PixelExtent, PixelRect,
    },
    ImageCaptureLimits, PlatformError,
};
use screencapturekit::{
    async_api::AsyncSCShareableContent,
    cm::FrameInfo,
    stream::{configuration::SCStreamConfiguration, content_filter::SCContentFilter},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

impl Capture for crate::MacOsPlatform {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        limits: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        request_index(requests)?;
        validate_limits(limits)?;
        if requests.is_empty() {
            return Ok(CaptureBatch::default());
        }
        let batch = match acquire(target, requests, limits) {
            Ok(batch) => batch,
            Err(crate::capture_stream::CaptureFailure::Unavailable(message)) => CaptureBatch {
                omissions: requests
                    .iter()
                    .map(|request| CaptureOmission {
                        attachment_id: request.id.clone(),
                        reason: CaptureOmissionReason::CaptureFailed,
                        detail: message.clone(),
                    })
                    .collect(),
                diagnostics: vec![message],
                ..CaptureBatch::default()
            },
            Err(error) => return Err(error.into_platform()),
        };
        validate_batch(batch, target, requests, limits)
    }
}

fn invalid(message: impl Into<String>) -> PlatformError {
    PlatformError::InvalidResponse(message.into())
}

fn validate_limits(limits: ImageCaptureLimits) -> Result<(), PlatformError> {
    if [
        limits.max_long_edge,
        limits.max_pixels,
        limits.max_attachment_bytes,
        limits.max_total_bytes,
    ]
    .contains(&0)
    {
        return Err(PlatformError::Operation(
            "image capture limits must be greater than zero".into(),
        ));
    }
    Ok(())
}

fn request_index(
    requests: &[CaptureRequest],
) -> Result<BTreeMap<&str, &CaptureRequest>, PlatformError> {
    let mut index = BTreeMap::new();
    for request in requests {
        if request.id.is_empty() || index.insert(request.id.as_str(), request).is_some() {
            return Err(PlatformError::Operation(
                "image capture request identities must be nonempty and unique".into(),
            ));
        }
        if !matches!(
            (&request.scope, &request.region),
            (CaptureScope::WindowFallback, CaptureRegion::FrameContent)
                | (
                    CaptureScope::AxElementRegion,
                    CaptureRegion::DesktopRegion { .. }
                )
        ) {
            return Err(PlatformError::Operation(
                "image capture scope and coordinate region disagree".into(),
            ));
        }
    }
    Ok(index)
}

type Failure = crate::capture_stream::CaptureFailure;
fn unavailable(error: impl std::fmt::Display) -> Failure {
    Failure::Unavailable(error.to_string())
}

fn acquire(
    target: CaptureTarget,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<CaptureBatch, Failure> {
    let (window, latch): (_, Option<Arc<AtomicBool>>) = match target {
        CaptureTarget::Registered(key) => {
            let (window, latch) =
                crate::source::capture_source(key.target).map_err(source_failure)?;
            (window, Some(latch))
        }
        CaptureTarget::Legacy { window_id } => {
            let content = crate::capture_stream::wait(
                AsyncSCShareableContent::get(),
                Duration::from_secs(15),
                "selected window lookup",
            )
            .map_err(unavailable)?
            .map_err(unavailable)?;
            let window = content
                .windows()
                .into_iter()
                .find(|window| window.window_id() == window_id)
                .ok_or_else(|| unavailable("the explicit selected window is unavailable"))?;
            (window, None)
        }
    };
    let filter = SCContentFilter::create()
        .with_window(&window)
        .build()
        .map_err(unavailable)?;
    // Filter geometry configures a bounded output, but never establishes the
    // acquired frame's desktop origin, scale, or content placement.
    let hint = filter.content_rect();
    let hint = Bounds {
        x: hint.origin.x,
        y: hint.origin.y,
        width: hint.size.width,
        height: hint.size.height,
    };
    if !valid_bounds(hint) {
        return Err(Failure::Invalid(
            "selected filter has invalid output sizing geometry".into(),
        ));
    }
    let scale = 2.0_f64.min(4096.0 / hint.width).min(4096.0 / hint.height);
    let configuration = SCStreamConfiguration::new()
        .with_captures_audio(false)
        .with_captures_microphone(false)
        .map_err(unavailable)?
        .with_width((hint.width * scale).floor().max(1.0) as u32)
        .with_height((hint.height * scale).floor().max(1.0) as u32)
        .with_scales_to_fit(true)
        .with_preserves_aspect_ratio(true)
        .map_err(unavailable)?
        .with_shows_cursor(false)
        .with_ignores_shadows_single_window(true)
        .map_err(unavailable)?
        .with_should_be_opaque(true)
        .map_err(unavailable)?;
    let before = screen_configuration();
    let sample = crate::capture_stream::acquire(&filter, &configuration, latch)?;
    let after = screen_configuration();
    let content_bounds = crate::capture_stream::required_content(&sample.info)?;
    let frame = AcquiredFrame {
        id: FrameId::try_from(uuid::Uuid::new_v4()).expect("new UUID is nonnil"),
        target,
        extent: PixelExtent {
            width: sample.bitmap.width(),
            height: sample.bitmap.height(),
        },
        content_bounds,
        desktop_mapping: desktop_mapping(&sample.info, &before, &after),
    };
    encode_frame(frame, &sample.bitmap, requests, limits)
}

fn source_failure(error: PlatformError) -> Failure {
    match error {
        PlatformError::CleanupUnconfirmed(message) => Failure::CleanupUnconfirmed(message),
        error => unavailable(error),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Screen {
    bounds: Bounds,
    scale: f64,
}

fn screen_configuration() -> Vec<Screen> {
    crate::source::on_main(|| {
        let marker =
            objc2::MainThreadMarker::new().expect("screen inventory executes on main thread");
        objc2_app_kit::NSScreen::screens(marker)
            .iter()
            .map(|screen| {
                let frame = screen.frame();
                Screen {
                    bounds: Bounds {
                        x: frame.origin.x,
                        y: frame.origin.y,
                        width: frame.size.width,
                        height: frame.size.height,
                    },
                    scale: screen.backingScaleFactor(),
                }
            })
            .collect()
    })
}

fn desktop_mapping(info: &FrameInfo, before: &[Screen], after: &[Screen]) -> DesktopMapping {
    let unknown = |reason| DesktopMapping::Unavailable { reason };
    // Evidence currently covers one zero-origin, 1x display only. Reading its
    // metadata does not prove the same units on Retina or multiple displays.
    if before != after
        || before.len() != 1
        || before[0].scale != 1.0
        || before[0].bounds.x != 0.0
        || before[0].bounds.y != 0.0
        || !valid_bounds(before[0].bounds)
    {
        return unknown(GeometryUnavailableReason::UnverifiedConfiguration);
    }
    let (Some(rect), Some(factor), Some(scale)) =
        (info.screen_rect, info.scale_factor, info.content_scale)
    else {
        return unknown(GeometryUnavailableReason::MissingMetadata);
    };
    let bounds = Bounds {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    };
    if !valid_bounds(bounds)
        || !factor.is_finite()
        || factor <= 0.0
        || !scale.is_finite()
        || scale <= 0.0
        || !(factor * scale).is_finite()
    {
        return unknown(GeometryUnavailableReason::InvalidMetadata);
    }
    if factor != 1.0 {
        return unknown(GeometryUnavailableReason::UnverifiedConfiguration);
    }
    let Ok(content) = crate::capture_stream::required_content(info) else {
        return unknown(GeometryUnavailableReason::InvalidMetadata);
    };
    if !valid_bounds(Bounds {
        x: content.x,
        y: content.y,
        width: bounds.width * factor * scale,
        height: bounds.height * factor * scale,
    }) {
        return unknown(GeometryUnavailableReason::InvalidMetadata);
    }
    DesktopMapping::Known {
        space: DesktopSpace::DesktopLogical,
        desktop_bounds: bounds,
        scale_x: factor * scale,
        scale_y: factor * scale,
    }
}

fn request_source(
    frame: &AcquiredFrame,
    request: &CaptureRequest,
) -> Result<Bounds, CaptureOmissionReason> {
    match request.region {
        CaptureRegion::FrameContent => Ok(frame.content_bounds),
        CaptureRegion::DesktopRegion {
            space,
            bounds,
            observed_window_frame,
        } => {
            if !valid_bounds(bounds) || !valid_bounds(observed_window_frame) {
                return Err(CaptureOmissionReason::InvalidBounds);
            }
            let DesktopMapping::Known {
                space: actual_space,
                desktop_bounds,
                ..
            } = frame.desktop_mapping
            else {
                return Err(CaptureOmissionReason::GeometryUnavailable);
            };
            if actual_space != space {
                return Err(CaptureOmissionReason::GeometryUnavailable);
            }
            if desktop_bounds != observed_window_frame {
                return Err(CaptureOmissionReason::ObservationChanged);
            }
            frame
                .desktop_to_frame(space, bounds)
                .ok_or(CaptureOmissionReason::InvalidBounds)
        }
    }
}

fn encode_frame(
    frame: AcquiredFrame,
    bitmap: &image::RgbaImage,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<CaptureBatch, Failure> {
    if !frame.is_valid()
        || frame.extent.width != bitmap.width()
        || frame.extent.height != bitmap.height()
    {
        return Err(Failure::Invalid(
            "native sample violates required frame geometry".into(),
        ));
    }
    let mut batch = CaptureBatch::default();
    let mut total_bytes = 0usize;
    for request in requests {
        let source = match request_source(&frame, request) {
            Ok(source) => source,
            Err(reason) => {
                omit(&mut batch, request, reason);
                continue;
            }
        };
        let Some(captured) = frame.clip(source) else {
            omit(&mut batch, request, CaptureOmissionReason::OutsideWindow);
            continue;
        };
        let Some(crop) = PixelRect::covering(captured, frame.extent) else {
            return Err(Failure::Invalid(
                "native content produced invalid integer crop".into(),
            ));
        };
        let cropped =
            image::imageops::crop_imm(bitmap, crop.x, crop.y, crop.width, crop.height).to_image();
        let (width, height) = bounded_dimensions(crop.width, crop.height, limits);
        let image = if (width, height) == (crop.width, crop.height) {
            cropped
        } else {
            image::imageops::resize(
                &cropped,
                width,
                height,
                image::imageops::FilterType::Lanczos3,
            )
        };
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                image.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| Failure::Invalid(error.to_string()))?;
        if png.len() > limits.max_attachment_bytes as usize
            || png.len() > (limits.max_total_bytes as usize).saturating_sub(total_bytes)
        {
            omit(&mut batch, request, CaptureOmissionReason::ByteBudget);
            continue;
        }
        total_bytes += png.len();
        batch.captures.push(CapturedImage {
            attachment_id: request.id.clone(),
            frame_id: frame.id,
            source_bounds: source,
            captured_bounds: captured,
            crop,
            coverage: if source == captured {
                CaptureCoverage::FullRegion
            } else {
                CaptureCoverage::VisibleSubregion
            },
            pixel_width: width as usize,
            pixel_height: height as usize,
            png,
        });
    }
    if !batch.captures.is_empty() {
        batch.frame = Some(frame);
    }
    Ok(batch)
}

fn omit(batch: &mut CaptureBatch, request: &CaptureRequest, reason: CaptureOmissionReason) {
    batch.omissions.push(CaptureOmission {
        attachment_id: request.id.clone(),
        reason,
        detail: format!("image region omitted: {reason:?}"),
    });
}

fn validate_batch(
    batch: CaptureBatch,
    target: CaptureTarget,
    requests: &[CaptureRequest],
    limits: ImageCaptureLimits,
) -> Result<CaptureBatch, PlatformError> {
    let index = request_index(requests)?;
    if batch.frame.is_some() != !batch.captures.is_empty()
        || batch
            .frame
            .as_ref()
            .is_some_and(|frame| !frame.is_valid() || frame.target != target)
    {
        return Err(invalid("image batch has invalid frame correlation"));
    }
    let mut resolved = BTreeSet::new();
    let mut total = 0usize;
    for image in &batch.captures {
        let request = index
            .get(image.attachment_id.as_str())
            .ok_or_else(|| invalid("unknown image attachment"))?;
        if !resolved.insert(image.attachment_id.as_str()) {
            return Err(invalid("duplicate image terminal outcome"));
        }
        let frame = batch
            .frame
            .as_ref()
            .ok_or_else(|| invalid("missing acquired frame"))?;
        let width =
            u32::try_from(image.pixel_width).map_err(|_| invalid("encoded width overflow"))?;
        let height =
            u32::try_from(image.pixel_height).map_err(|_| invalid("encoded height overflow"))?;
        if image.frame_id != frame.id
            || (width, height) != bounded_dimensions(image.crop.width, image.crop.height, limits)
            || request_source(frame, request).ok() != Some(image.source_bounds)
            || !frame.validates_image(
                image.source_bounds,
                image.captured_bounds,
                image.crop,
                PixelExtent { width, height },
            )
            || image.coverage
                != if image.source_bounds == image.captured_bounds {
                    CaptureCoverage::FullRegion
                } else {
                    CaptureCoverage::VisibleSubregion
                }
            || width.max(height) > limits.max_long_edge
            || u64::from(width) * u64::from(height) > u64::from(limits.max_pixels)
            || image.png.len() > limits.max_attachment_bytes as usize
            || image.png.len() > (limits.max_total_bytes as usize).saturating_sub(total)
        {
            return Err(invalid(
                "image violates request, crop, dimensions or finite budgets",
            ));
        }
        total += image.png.len();
    }
    for omission in &batch.omissions {
        if !index.contains_key(omission.attachment_id.as_str())
            || !resolved.insert(omission.attachment_id.as_str())
        {
            return Err(invalid("invalid image omission identity"));
        }
    }
    if resolved.len() != requests.len() {
        return Err(invalid("image batch omitted a terminal outcome"));
    }
    Ok(batch)
}

fn bounded_dimensions(width: u32, height: u32, limits: ImageCaptureLimits) -> (u32, u32) {
    let scale = 1.0_f64
        .min(f64::from(limits.max_long_edge) / f64::from(width.max(height)))
        .min((f64::from(limits.max_pixels) / (f64::from(width) * f64::from(height))).sqrt());
    let mut width = (f64::from(width) * scale).floor().max(1.0) as u32;
    let mut height = (f64::from(height) * scale).floor().max(1.0) as u32;
    if u64::from(width) * u64::from(height) > u64::from(limits.max_pixels) {
        if width >= height {
            width = (limits.max_pixels / height).max(1);
        } else {
            height = (limits.max_pixels / width).max(1);
        }
    }
    (width, height)
}

pub(super) fn unpremultiply_rgba(pixels: &mut [u8]) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_cleanup_failure_keeps_its_fatal_category_before_native_start() {
        assert!(matches!(
            source_failure(PlatformError::CleanupUnconfirmed("Stop unknown".into())),
            Failure::CleanupUnconfirmed(_)
        ));
        assert!(matches!(
            source_failure(PlatformError::Operation("unavailable".into())),
            Failure::Unavailable(_)
        ));
    }
    #[test]
    fn desktop_mapping_is_optional_and_only_verified_configuration_is_known() {
        use screencapturekit::cg::{CGPoint, CGRect, CGSize};
        let rect = |x, y, width, height| CGRect {
            origin: CGPoint { x, y },
            size: CGSize { width, height },
        };
        let screen = Screen {
            bounds: bounds(0.0, 0.0, 3840.0, 2160.0),
            scale: 1.0,
        };
        let info = FrameInfo {
            screen_rect: Some(rect(-12.0, 80.0, 1200.0, 800.0)),
            content_rect: Some(rect(0.0, 0.0, 1200.0, 800.0)),
            scale_factor: Some(1.0),
            content_scale: Some(1.0),
            ..Default::default()
        };
        let screens = vec![screen.clone()];
        assert!(matches!(
            desktop_mapping(&info, &screens, &screens),
            DesktopMapping::Known { .. }
        ));
        for changed in [
            vec![],
            vec![Screen {
                scale: 2.0,
                ..screen.clone()
            }],
            vec![screen.clone(), screen.clone()],
            vec![Screen {
                bounds: bounds(-3840.0, 0.0, 3840.0, 2160.0),
                ..screen.clone()
            }],
        ] {
            assert_eq!(
                desktop_mapping(&info, &changed, &changed),
                DesktopMapping::Unavailable {
                    reason: GeometryUnavailableReason::UnverifiedConfiguration
                }
            );
            assert!(matches!(
                desktop_mapping(&info, &screens, &changed),
                DesktopMapping::Unavailable { .. }
            ));
        }
        let missing = FrameInfo {
            screen_rect: None,
            ..info.clone()
        };
        assert_eq!(
            desktop_mapping(&missing, &screens, &screens),
            DesktopMapping::Unavailable {
                reason: GeometryUnavailableReason::MissingMetadata
            }
        );
        let invalid = FrameInfo {
            content_scale: Some(f64::MAX),
            ..info
        };
        assert_eq!(
            desktop_mapping(&invalid, &screens, &screens),
            DesktopMapping::Unavailable {
                reason: GeometryUnavailableReason::InvalidMetadata
            }
        );
    }
    #[test]
    fn premultiplied_rgba_is_encoded_with_straight_png_alpha() {
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
    fn bounds(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
        }
    }
    fn limits() -> ImageCaptureLimits {
        ImageCaptureLimits {
            max_long_edge: 10,
            max_pixels: 100,
            max_attachment_bytes: 4096,
            max_total_bytes: 4096,
        }
    }
    fn frame() -> AcquiredFrame {
        AcquiredFrame {
            id: FrameId::try_from(uuid::Uuid::new_v4()).unwrap(),
            target: CaptureTarget::Legacy { window_id: 1 },
            extent: PixelExtent {
                width: 20,
                height: 20,
            },
            content_bounds: bounds(0.25, 0.5, 20.0, 19.5),
            desktop_mapping: DesktopMapping::Known {
                space: DesktopSpace::DesktopLogical,
                desktop_bounds: bounds(-10.0, 30.0, 10.0, 9.75),
                scale_x: 2.0,
                scale_y: 2.0,
            },
        }
    }
    fn requests() -> Vec<CaptureRequest> {
        vec![CaptureRequest {
            id: "image".into(),
            scope: CaptureScope::WindowFallback,
            region: CaptureRegion::FrameContent,
        }]
    }
    fn batch() -> CaptureBatch {
        encode_frame(
            frame(),
            &image::RgbaImage::new(20, 20),
            &requests(),
            limits(),
        )
        .unwrap()
    }
    #[test]
    fn fractional_content_rounds_outward_and_downsizes_actual_pixels() {
        let batch = batch();
        let image = &batch.captures[0];
        assert_eq!(
            image.crop,
            PixelRect {
                x: 0,
                y: 0,
                width: 20,
                height: 20
            }
        );
        assert_eq!(image.coverage, CaptureCoverage::VisibleSubregion);
        assert_eq!((image.pixel_width, image.pixel_height), (10, 10));
        validate_batch(
            batch,
            CaptureTarget::Legacy { window_id: 1 },
            &requests(),
            limits(),
        )
        .unwrap();
    }
    #[test]
    fn unknown_mapping_keeps_frame_content_but_omits_desktop_and_changed_observation() {
        let mut f = frame();
        let desktop = CaptureRequest {
            id: "node".into(),
            scope: CaptureScope::AxElementRegion,
            region: CaptureRegion::DesktopRegion {
                space: DesktopSpace::DesktopLogical,
                bounds: bounds(-9.0, 31.0, 1.0, 1.0),
                observed_window_frame: bounds(-10.0, 30.0, 10.0, 9.75),
            },
        };
        assert!(request_source(&f, &desktop).is_ok());
        if let DesktopMapping::Known { desktop_bounds, .. } = &mut f.desktop_mapping {
            desktop_bounds.x += 1.0;
        }
        assert_eq!(
            request_source(&f, &desktop),
            Err(CaptureOmissionReason::ObservationChanged)
        );
        f.desktop_mapping = DesktopMapping::Unavailable {
            reason: GeometryUnavailableReason::UnverifiedConfiguration,
        };
        assert_eq!(
            request_source(&f, &desktop),
            Err(CaptureOmissionReason::GeometryUnavailable)
        );
        assert!(request_source(&f, &requests()[0]).is_ok());
    }

    #[test]
    fn negative_desktop_origin_and_partial_node_crop_use_actual_frame_pixels() {
        let request = CaptureRequest {
            id: "node".into(),
            scope: CaptureScope::AxElementRegion,
            region: CaptureRegion::DesktopRegion {
                space: DesktopSpace::DesktopLogical,
                bounds: bounds(-11.0, 31.0, 4.0, 4.0),
                observed_window_frame: bounds(-10.0, 30.0, 10.0, 9.75),
            },
        };
        let b = encode_frame(
            frame(),
            &image::RgbaImage::new(20, 20),
            std::slice::from_ref(&request),
            limits(),
        )
        .unwrap();
        let image = &b.captures[0];
        assert_eq!(image.source_bounds, bounds(-1.75, 2.5, 8.0, 8.0));
        assert_eq!(image.captured_bounds, bounds(0.25, 2.5, 6.0, 8.0));
        assert_eq!(
            image.crop,
            PixelRect {
                x: 0,
                y: 2,
                width: 7,
                height: 9
            }
        );
        assert_eq!(image.coverage, CaptureCoverage::VisibleSubregion);
        assert_eq!(
            image::load_from_memory(&image.png)
                .unwrap()
                .into_rgba8()
                .dimensions(),
            (7, 9)
        );
        validate_batch(
            b,
            CaptureTarget::Legacy { window_id: 1 },
            &[request],
            limits(),
        )
        .unwrap();
    }
    #[test]
    fn typed_batch_rejects_uncorrelated_geometry_dimensions_and_terminal_outcomes() {
        for mutation in 0..10 {
            let mut b = batch();
            match mutation {
                0 => b.frame.as_mut().unwrap().target = CaptureTarget::Legacy { window_id: 2 },
                1 => b.captures[0].frame_id = FrameId::try_from(uuid::Uuid::new_v4()).unwrap(),
                2 => b.captures[0].crop.x = 1,
                3 => b.captures[0].source_bounds.width = f64::NAN,
                4 => b.captures[0].pixel_width = 9,
                5 => b.captures[0].coverage = CaptureCoverage::FullRegion,
                6 => b.captures.push(b.captures[0].clone()),
                7 => b.captures[0].attachment_id = "unknown".into(),
                8 => b.frame = None,
                _ => {
                    b.captures.clear();
                    b.frame = None;
                }
            }
            assert!(
                validate_batch(
                    b,
                    CaptureTarget::Legacy { window_id: 1 },
                    &requests(),
                    limits()
                )
                .is_err(),
                "mutation {mutation}"
            );
        }
    }
    #[test]
    fn no_published_image_has_no_frame_and_required_invalid_geometry_still_errors() {
        let mut tiny = limits();
        tiny.max_attachment_bytes = 1;
        let b = encode_frame(frame(), &image::RgbaImage::new(20, 20), &requests(), tiny).unwrap();
        assert!(b.frame.is_none());
        assert_eq!(b.omissions[0].reason, CaptureOmissionReason::ByteBudget);
        let mut bad = frame();
        bad.content_bounds.width = f64::INFINITY;
        assert!(matches!(
            encode_frame(bad, &image::RgbaImage::new(20, 20), &requests(), limits()),
            Err(Failure::Invalid(_))
        ));
    }
    #[test]
    fn one_pixel_budget_and_extreme_aspect_ratio_remain_valid() {
        let mut l = limits();
        l.max_pixels = 1;
        assert_eq!(bounded_dimensions(4096, 1, l), (1, 1));
        let f = AcquiredFrame {
            extent: PixelExtent {
                width: 1,
                height: 1,
            },
            content_bounds: bounds(0.0, 0.0, 1.0, 1.0),
            ..frame()
        };
        let b = encode_frame(f, &image::RgbaImage::new(1, 1), &requests(), l).unwrap();
        validate_batch(b, CaptureTarget::Legacy { window_id: 1 }, &requests(), l).unwrap();
    }
    #[test]
    fn scope_region_and_duplicate_ids_are_explicit_request_errors() {
        let mut r = requests();
        r[0].scope = CaptureScope::AxElementRegion;
        assert!(request_index(&r).is_err());
        let mut r = requests();
        r.push(r[0].clone());
        assert!(request_index(&r).is_err());
    }
}
