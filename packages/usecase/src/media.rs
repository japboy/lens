use crate::platform::{
    bounds_from_platform as to_domain_bounds, bounds_to_platform as to_platform_bounds,
};
use base64::prelude::*;
use domain::lens::{
    LensMediaAttachment, LensMediaCapture, LensMediaCoverage, LensMediaGeometry, LensMediaOmission,
    LensMediaOmissionReason, LensMediaPayload, LensMediaPlan, LensMediaScope,
};
use port_platform::{
    capture::{
        Capture, CaptureBatch, CaptureCoverage, CaptureOmissionReason, CaptureRegion,
        CaptureRequest, CaptureScope, CaptureTarget,
    },
    ImageCaptureLimits, PlatformError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
};
use uuid::Uuid;

/// Domain publication authority is application-owned, never a native capture argument.
pub struct MediaCaptureContext {
    pub target: CaptureTarget,
    pub target_id: String,
    pub context_id: Uuid,
    pub context_revision: NonZeroU64,
    pub desktop_observation: Option<DesktopObservation>,
}

/// Earlier tree geometry is an explicit correlated observation, not an image snapshot.
#[derive(Debug, Clone, Copy)]
pub struct DesktopObservation {
    pub space: domain::geometry::DesktopSpace,
    pub window_frame: domain::model::Bounds,
}

pub fn capture_media(
    capability: &dyn Capture,
    context: MediaCaptureContext,
    mut plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    if plan.requests.is_empty() {
        return Ok(LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        });
    }
    if plan
        .requests
        .iter()
        .map(|request| &request.id)
        .collect::<BTreeSet<_>>()
        .len()
        != plan.requests.len()
    {
        return Err(PlatformError::Operation(
            "image capture request identities must be unique".into(),
        ));
    }
    let mut requests = Vec::new();
    let mut admitted = Vec::new();
    for request in plan.requests {
        let (scope, region) = match request.scope {
            LensMediaScope::WindowFallback => {
                (CaptureScope::WindowFallback, CaptureRegion::FrameContent)
            }
            LensMediaScope::AxElementRegion => {
                match (request.bounds, context.desktop_observation) {
                    (Some(bounds), Some(observation)) => (
                        CaptureScope::AxElementRegion,
                        CaptureRegion::DesktopRegion {
                            space: to_platform_space(observation.space),
                            bounds: to_platform_bounds(bounds),
                            observed_window_frame: to_platform_bounds(observation.window_frame),
                        },
                    ),
                    _ => {
                        plan.omissions.push(LensMediaOmission {
                        target_id: context.target_id.clone(), attachment_id: Some(request.id),
                        source_node_id: request.source_node_id,
                        reason: LensMediaOmissionReason::GeometryUnavailable, omitted_count: 1,
                        first_order: None, last_order: None,
                        detail: "Image-region capture has no explicit tree coordinate-space observation.".into(),
                    });
                        continue;
                    }
                }
            }
        };
        requests.push(CaptureRequest {
            id: request.id.clone(),
            scope,
            region,
        });
        admitted.push(request);
    }
    plan.requests = admitted;
    if requests.is_empty() {
        return Ok(LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        });
    }
    let batch = capability.capture(context.target, &requests, limits)?;
    assemble_media(context, plan, batch, limits)
}

fn to_platform_space(
    space: domain::geometry::DesktopSpace,
) -> port_platform::geometry::DesktopSpace {
    match space {
        domain::geometry::DesktopSpace::DesktopLogical => {
            port_platform::geometry::DesktopSpace::DesktopLogical
        }
        domain::geometry::DesktopSpace::DesktopPhysicalPixels => {
            port_platform::geometry::DesktopSpace::DesktopPhysicalPixels
        }
    }
}

fn to_domain_coverage(coverage: CaptureCoverage) -> LensMediaCoverage {
    match coverage {
        CaptureCoverage::FullRegion => LensMediaCoverage::FullRegion,
        CaptureCoverage::VisibleSubregion => LensMediaCoverage::VisibleSubregion,
    }
}

fn to_domain_omission(reason: CaptureOmissionReason) -> LensMediaOmissionReason {
    match reason {
        CaptureOmissionReason::MissingBounds => LensMediaOmissionReason::MissingBounds,
        CaptureOmissionReason::InvalidBounds => LensMediaOmissionReason::InvalidBounds,
        CaptureOmissionReason::OutsideWindow => LensMediaOmissionReason::OutsideWindow,
        CaptureOmissionReason::AttachmentLimit => LensMediaOmissionReason::AttachmentLimit,
        CaptureOmissionReason::ByteBudget => LensMediaOmissionReason::ByteBudget,
        CaptureOmissionReason::CaptureFailed => LensMediaOmissionReason::CaptureFailed,
        CaptureOmissionReason::GeometryUnavailable => LensMediaOmissionReason::GeometryUnavailable,
        CaptureOmissionReason::ObservationChanged => LensMediaOmissionReason::ObservationChanged,
    }
}

fn assemble_media(
    context: MediaCaptureContext,
    plan: LensMediaPlan,
    batch: CaptureBatch,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    let requests_by_id = plan
        .requests
        .iter()
        .map(|request| (request.id.as_str(), request))
        .collect::<BTreeMap<_, _>>();
    if batch.frame.is_some() && batch.captures.is_empty() {
        return Err(PlatformError::InvalidResponse(
            "capture frame has no captured image".into(),
        ));
    }
    let frame = batch
        .frame
        .as_ref()
        .map(|frame| {
            if frame.target != context.target || !frame.is_valid() {
                return Err(PlatformError::InvalidResponse(
                    "capture frame does not match the requested read or valid geometry".into(),
                ));
            }
            Ok(crate::platform::frame_from_platform(frame))
        })
        .transpose()?;
    let mut result = LensMediaCapture {
        omissions: plan.omissions,
        diagnostics: batch.diagnostics,
        ..LensMediaCapture::default()
    };
    let mut resolved = BTreeSet::new();
    let mut total_bytes = 0usize;
    for capture in batch.captures {
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
        let frame = frame.as_ref().ok_or_else(|| {
            PlatformError::InvalidResponse("captured image has no acquired frame".into())
        })?;
        if Uuid::from(capture.frame_id) != Uuid::from(frame.id) {
            return Err(PlatformError::InvalidResponse(
                "captured image belongs to another frame".into(),
            ));
        }
        let source_bounds = to_domain_bounds(capture.source_bounds);
        let captured_bounds = to_domain_bounds(capture.captured_bounds);
        let crop = domain::geometry::PixelRect {
            x: capture.crop.x,
            y: capture.crop.y,
            width: capture.crop.width,
            height: capture.crop.height,
        };
        let encoded = domain::geometry::PixelExtent {
            width: u32::try_from(capture.pixel_width)
                .map_err(|_| PlatformError::InvalidResponse("encoded width exceeds u32".into()))?,
            height: u32::try_from(capture.pixel_height)
                .map_err(|_| PlatformError::InvalidResponse("encoded height exceeds u32".into()))?,
        };
        let expected_source = match request.scope {
            LensMediaScope::WindowFallback => Some(frame.content_bounds),
            LensMediaScope::AxElementRegion => {
                context.desktop_observation.and_then(|observation| {
                    let domain::geometry::DesktopMapping::Known {
                        space,
                        desktop_bounds,
                        ..
                    } = frame.desktop_mapping
                    else {
                        return None;
                    };
                    if space != observation.space || desktop_bounds != observation.window_frame {
                        return None;
                    }
                    frame.desktop_to_frame(observation.space, request.bounds?)
                })
            }
        };
        if expected_source != Some(source_bounds)
            || !frame.validates_image(source_bounds, captured_bounds, crop, encoded)
            || (capture.coverage == CaptureCoverage::FullRegion)
                != (source_bounds == captured_bounds)
        {
            return Err(PlatformError::InvalidResponse(
                "captured image geometry disagrees with its request, frame, crop, or coverage"
                    .into(),
            ));
        }
        total_bytes = total_bytes
            .checked_add(capture.png.len())
            .ok_or_else(|| PlatformError::InvalidResponse("media byte count overflow".into()))?;
        if total_bytes > limits.max_total_bytes as usize {
            return Err(PlatformError::InvalidResponse(
                "captured images exceed total byte limit".into(),
            ));
        }
        validate_png(&capture.png, encoded, limits)?;
        let uri = media_uri(
            context.context_id,
            context.context_revision.get(),
            &capture.attachment_id,
        );
        result.attachments.push(LensMediaAttachment {
            id: capture.attachment_id.clone(),
            target_id: context.target_id.clone(),
            uri: uri.clone(),
            scope: request.scope,
            source_node_id: request.source_node_id.clone(),
            geometry: LensMediaGeometry::Acquired {
                frame: frame.clone(),
                source_bounds,
                captured_bounds,
                crop,
            },
            coverage: to_domain_coverage(capture.coverage),
            mime_type: "image/png".into(),
            pixel_width: capture.pixel_width,
            pixel_height: capture.pixel_height,
            encoded_bytes: capture.png.len(),
        });
        result.payloads.push(LensMediaPayload {
            attachment_id: capture.attachment_id,
            uri,
            mime_type: "image/png".into(),
            data: BASE64_STANDARD.encode(capture.png),
        });
    }
    for omission in batch.omissions {
        let request = requests_by_id
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
        result.omissions.push(LensMediaOmission {
            target_id: context.target_id.clone(),
            attachment_id: Some(omission.attachment_id),
            source_node_id: request.source_node_id.clone(),
            reason: to_domain_omission(omission.reason),
            omitted_count: 1,
            first_order: None,
            last_order: None,
            detail: omission.detail,
        });
    }
    for request in &plan.requests {
        if !resolved.contains(&request.id) {
            result.omissions.push(LensMediaOmission {
                target_id: context.target_id.clone(),
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

fn validate_png(
    png: &[u8],
    encoded: domain::geometry::PixelExtent,
    limits: ImageCaptureLimits,
) -> Result<(), PlatformError> {
    let invalid = |detail: String| {
        PlatformError::InvalidResponse(format!("captured PNG is invalid: {detail}"))
    };
    if !encoded.is_valid()
        || encoded.width.max(encoded.height) > limits.max_long_edge
        || u64::from(encoded.width) * u64::from(encoded.height) > u64::from(limits.max_pixels)
        || png.len() > limits.max_attachment_bytes as usize
        || png.len() > limits.max_total_bytes as usize
    {
        return Err(invalid(
            "declared dimensions or bytes exceed finite limits".into(),
        ));
    }
    let reader = || {
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let mut decode_limits = image::Limits::default();
        decode_limits.max_image_width = Some(limits.max_long_edge);
        decode_limits.max_image_height = Some(limits.max_long_edge);
        decode_limits.max_alloc = Some(u64::from(limits.max_pixels) * 8);
        reader.limits(decode_limits);
        reader
    };
    let actual = reader()
        .into_dimensions()
        .map_err(|e| invalid(e.to_string()))?;
    if actual != (encoded.width, encoded.height) {
        return Err(invalid("encoded dimensions disagree with metadata".into()));
    }
    reader().decode().map_err(|e| invalid(e.to_string()))?;
    Ok(())
}

fn media_uri(context_id: Uuid, context_revision: u64, attachment_id: &str) -> String {
    format!("lens://context/{context_id}/{context_revision}/media/{attachment_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::lens::LensMediaRequest;
    use domain::model::Bounds;
    use port_platform::capture::{self, CaptureOmission, CapturedImage};
    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedCapture {
        calls: Mutex<Vec<(CaptureTarget, Vec<CaptureRequest>)>>,
        batch: CaptureBatch,
        failure: Option<String>,
    }

    impl Capture for ScriptedCapture {
        fn capture(
            &self,
            target: CaptureTarget,
            requests: &[CaptureRequest],
            _limits: ImageCaptureLimits,
        ) -> Result<CaptureBatch, PlatformError> {
            self.calls.lock().unwrap().push((target, requests.to_vec()));
            match &self.failure {
                Some(message) => Err(PlatformError::Operation(message.clone())),
                None => Ok(self.batch.clone()),
            }
        }
    }

    fn context() -> MediaCaptureContext {
        MediaCaptureContext {
            target: CaptureTarget::Registered(port_platform::authority::TargetReadKey {
                target: port_platform::authority::TargetAuthority {
                    operation_id: Uuid::from_u128(11),
                    receipt: Uuid::from_u128(22).try_into().unwrap(),
                },
                sequence: port_platform::authority::ReadSequence::FIRST,
            }),
            target_id: "window-17".into(),
            context_id: Uuid::from_u128(42),
            context_revision: NonZeroU64::new(7).unwrap(),
            desktop_observation: Some(DesktopObservation {
                space: domain::geometry::DesktopSpace::DesktopLogical,
                window_frame: Bounds {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
            }),
        }
    }

    fn limits() -> ImageCaptureLimits {
        ImageCaptureLimits {
            max_long_edge: 10,
            max_pixels: 100,
            max_attachment_bytes: 1024,
            max_total_bytes: 2048,
        }
    }

    fn plan() -> LensMediaPlan {
        LensMediaPlan {
            requests: vec![LensMediaRequest {
                id: "media-1".into(),
                scope: LensMediaScope::AxElementRegion,
                source_node_id: Some("node-1".into()),
                bounds: Some(Bounds {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                }),
            }],
            omissions: Vec::new(),
        }
    }

    fn batch() -> CaptureBatch {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0; 3 * 4 * 4], 3, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        let bounds = capture::Bounds {
            x: 0.0,
            y: 0.0,
            width: 3.0,
            height: 4.0,
        };
        CaptureBatch {
            frame: Some(port_platform::geometry::AcquiredFrame {
                id: Uuid::from_u128(33).try_into().unwrap(),
                target: context().target,
                extent: port_platform::geometry::PixelExtent {
                    width: 3,
                    height: 4,
                },
                content_bounds: bounds,
                desktop_mapping: port_platform::geometry::DesktopMapping::Known {
                    space: port_platform::geometry::DesktopSpace::DesktopLogical,
                    desktop_bounds: to_platform_bounds(
                        context().desktop_observation.unwrap().window_frame,
                    ),
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
            }),
            captures: vec![CapturedImage {
                attachment_id: "media-1".into(),
                frame_id: Uuid::from_u128(33).try_into().unwrap(),
                source_bounds: bounds,
                captured_bounds: bounds,
                crop: port_platform::geometry::PixelRect {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 4,
                },
                coverage: CaptureCoverage::FullRegion,
                pixel_width: 3,
                pixel_height: 4,
                png,
            }],
            omissions: vec![],
            diagnostics: vec!["capture diagnostic".into()],
        }
    }

    #[test]
    fn registered_capture_keeps_domain_authority_and_native_facts_separate() {
        let capability = ScriptedCapture {
            batch: batch(),
            ..Default::default()
        };
        let result = capture_media(&capability, context(), plan(), limits()).unwrap();
        assert_eq!(
            *capability.calls.lock().unwrap(),
            vec![(
                context().target,
                vec![CaptureRequest {
                    id: "media-1".into(),
                    scope: CaptureScope::AxElementRegion,
                    region: CaptureRegion::DesktopRegion {
                        space: port_platform::geometry::DesktopSpace::DesktopLogical,
                        bounds: to_platform_bounds(plan().requests[0].bounds.unwrap()),
                        observed_window_frame: to_platform_bounds(
                            context().desktop_observation.unwrap().window_frame
                        ),
                    },
                }]
            )]
        );
        assert_eq!(result.diagnostics, ["capture diagnostic"]);
        let attachment = &result.attachments[0];
        assert_eq!(
            attachment.uri,
            "lens://context/00000000-0000-0000-0000-00000000002a/7/media/media-1"
        );
        assert!(
            matches!(&attachment.geometry, LensMediaGeometry::Acquired { frame, crop, .. }
            if Uuid::from(frame.id) == Uuid::from_u128(33) && crop.width == 3 && crop.height == 4)
        );
        assert!(!serde_json::to_string(&attachment.geometry)
            .unwrap()
            .contains("receipt"));
        assert_eq!(
            result.payloads,
            [LensMediaPayload {
                attachment_id: "media-1".into(),
                uri: result.attachments[0].uri.clone(),
                mime_type: "image/png".into(),
                data: BASE64_STANDARD.encode(&batch().captures[0].png),
            }]
        );
        assert!(result.omissions.is_empty());
    }

    #[test]
    fn empty_and_duplicate_plans_never_invoke_capture() {
        let capability = ScriptedCapture::default();
        assert_eq!(
            capture_media(&capability, context(), LensMediaPlan::default(), limits()).unwrap(),
            LensMediaCapture::default()
        );
        let mut duplicate = plan();
        duplicate.requests.push(duplicate.requests[0].clone());
        assert!(matches!(
            capture_media(&capability, context(), duplicate, limits()),
            Err(PlatformError::Operation(_))
        ));
        assert!(capability.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn platform_failure_is_not_an_empty_success() {
        let capability = ScriptedCapture {
            failure: Some("capture unavailable".into()),
            ..Default::default()
        };
        assert_eq!(
            capture_media(&capability, context(), plan(), limits())
                .unwrap_err()
                .to_string(),
            "platform operation failed: capture unavailable"
        );
        assert_eq!(capability.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn omitted_and_missing_outcomes_keep_source_links_and_plan_order() {
        let mut plan = plan();
        let mut missing = plan.requests[0].clone();
        missing.id = "media-2".into();
        missing.source_node_id = Some("node-2".into());
        plan.requests.push(missing);
        let prior = LensMediaOmission {
            target_id: "window-17".into(),
            attachment_id: None,
            source_node_id: None,
            reason: LensMediaOmissionReason::AttachmentLimit,
            omitted_count: 2,
            first_order: Some(5),
            last_order: Some(6),
            detail: "planner limit".into(),
        };
        plan.omissions.push(prior.clone());
        let capability = ScriptedCapture {
            batch: CaptureBatch {
                omissions: vec![CaptureOmission {
                    attachment_id: "media-1".into(),
                    reason: CaptureOmissionReason::OutsideWindow,
                    detail: "outside".into(),
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let result = capture_media(&capability, context(), plan, limits()).unwrap();
        assert!(result.attachments.is_empty());
        assert_eq!(
            result.omissions,
            vec![
                prior,
                LensMediaOmission {
                    target_id: "window-17".into(),
                    attachment_id: Some("media-1".into()),
                    source_node_id: Some("node-1".into()),
                    reason: LensMediaOmissionReason::OutsideWindow,
                    omitted_count: 1,
                    first_order: None,
                    last_order: None,
                    detail: "outside".into(),
                },
                LensMediaOmission {
                    target_id: "window-17".into(),
                    attachment_id: Some("media-2".into()),
                    source_node_id: Some("node-2".into()),
                    reason: LensMediaOmissionReason::CaptureFailed,
                    omitted_count: 1,
                    first_order: None,
                    last_order: None,
                    detail: "Native image capture returned no terminal outcome for this request."
                        .into(),
                }
            ]
        );
    }

    #[test]
    fn invalid_terminal_correlations_are_rejected_before_publication() {
        let mut unknown = batch();
        unknown.captures[0].attachment_id = "unknown".into();
        let mut duplicate = batch();
        duplicate.captures.push(duplicate.captures[0].clone());
        let mut conflicting = batch();
        conflicting.omissions.push(CaptureOmission {
            attachment_id: "media-1".into(),
            reason: CaptureOmissionReason::CaptureFailed,
            detail: "conflict".into(),
        });
        let mut unknown_omission = CaptureBatch::default();
        unknown_omission.omissions.push(CaptureOmission {
            attachment_id: "unknown".into(),
            reason: CaptureOmissionReason::CaptureFailed,
            detail: "unknown".into(),
        });
        for batch in [unknown, duplicate, conflicting, unknown_omission] {
            let capability = ScriptedCapture {
                batch,
                ..Default::default()
            };
            assert!(matches!(
                capture_media(&capability, context(), plan(), limits()),
                Err(PlatformError::InvalidResponse(_))
            ));
        }
    }

    #[test]
    fn compatibility_variants_convert_exhaustively_without_wire_changes() {
        for (native, domain) in [
            (
                CaptureOmissionReason::MissingBounds,
                LensMediaOmissionReason::MissingBounds,
            ),
            (
                CaptureOmissionReason::InvalidBounds,
                LensMediaOmissionReason::InvalidBounds,
            ),
            (
                CaptureOmissionReason::OutsideWindow,
                LensMediaOmissionReason::OutsideWindow,
            ),
            (
                CaptureOmissionReason::AttachmentLimit,
                LensMediaOmissionReason::AttachmentLimit,
            ),
            (
                CaptureOmissionReason::ByteBudget,
                LensMediaOmissionReason::ByteBudget,
            ),
            (
                CaptureOmissionReason::CaptureFailed,
                LensMediaOmissionReason::CaptureFailed,
            ),
            (
                CaptureOmissionReason::GeometryUnavailable,
                LensMediaOmissionReason::GeometryUnavailable,
            ),
            (
                CaptureOmissionReason::ObservationChanged,
                LensMediaOmissionReason::ObservationChanged,
            ),
        ] {
            assert_eq!(to_domain_omission(native), domain);
            assert_eq!(
                serde_json::to_value(native).unwrap(),
                serde_json::to_value(domain).unwrap()
            );
        }
        assert_eq!(
            to_domain_coverage(CaptureCoverage::VisibleSubregion),
            LensMediaCoverage::VisibleSubregion
        );
        let capability = ScriptedCapture::default();
        let mut legacy = context();
        legacy.target = CaptureTarget::Legacy { window_id: 17 };
        let mut fallback = plan();
        fallback.requests[0].scope = LensMediaScope::WindowFallback;
        fallback.requests[0].bounds = None;
        capture_media(&capability, legacy, fallback, limits()).unwrap();
        assert_eq!(
            capability.calls.lock().unwrap()[0],
            (
                CaptureTarget::Legacy { window_id: 17 },
                vec![CaptureRequest {
                    id: "media-1".into(),
                    scope: CaptureScope::WindowFallback,
                    region: CaptureRegion::FrameContent,
                }]
            )
        );
    }

    #[test]
    fn registered_media_uri_owns_the_exact_context_revision() {
        let context_id = Uuid::from_u128(42);
        assert_eq!(
            media_uri(context_id, 7, "media-1"),
            "lens://context/00000000-0000-0000-0000-00000000002a/7/media/media-1"
        );
    }

    #[test]
    fn fake_providers_cannot_publish_self_consistent_but_unrequested_or_invalid_images() {
        let mut wrong_read = batch();
        if let CaptureTarget::Registered(mut key) = wrong_read.frame.as_ref().unwrap().target {
            key.sequence = "2".to_owned().try_into().unwrap();
            wrong_read.frame.as_mut().unwrap().target = CaptureTarget::Registered(key);
        }
        let mut wrong_frame = batch();
        wrong_frame.captures[0].frame_id = Uuid::from_u128(99).try_into().unwrap();
        let mut wrong_region = batch();
        wrong_region.captures[0].source_bounds.x = 0.5;
        wrong_region.captures[0].source_bounds.width = 2.0;
        wrong_region.captures[0].captured_bounds = wrong_region.captures[0].source_bounds;
        // It remains internally valid, so only reconstruction of the ORIGINAL request rejects it.
        let image = &wrong_region.captures[0];
        let frame = crate::platform::frame_from_platform(wrong_region.frame.as_ref().unwrap());
        assert!(frame.validates_image(
            to_domain_bounds(image.source_bounds),
            to_domain_bounds(image.captured_bounds),
            domain::geometry::PixelRect {
                x: image.crop.x,
                y: image.crop.y,
                width: image.crop.width,
                height: image.crop.height
            },
            domain::geometry::PixelExtent {
                width: 3,
                height: 4
            }
        ));
        let mut changed_observation = batch();
        if let port_platform::geometry::DesktopMapping::Known { desktop_bounds, .. } =
            &mut changed_observation.frame.as_mut().unwrap().desktop_mapping
        {
            desktop_bounds.x += 1.0;
        }
        let mut wrong_units = batch();
        if let port_platform::geometry::DesktopMapping::Known { space, .. } =
            &mut wrong_units.frame.as_mut().unwrap().desktop_mapping
        {
            *space = port_platform::geometry::DesktopSpace::DesktopPhysicalPixels;
        }
        let mut wrong_dimensions = batch();
        wrong_dimensions.captures[0].pixel_width = 4;
        let mut malformed = batch();
        malformed.captures[0].png = vec![1, 2, 3];
        let mut truncated = batch();
        truncated.captures[0].png.truncate(33);
        let mut oversized = batch();
        oversized.captures[0].png = vec![0; limits().max_attachment_bytes as usize + 1];
        let mut invalid_crop = batch();
        invalid_crop.captures[0].crop.x = 1;
        let mut no_frame = batch();
        no_frame.frame = None;
        let mut empty_frame = batch();
        empty_frame.captures.clear();
        for batch in [
            wrong_read,
            wrong_frame,
            wrong_region,
            changed_observation,
            wrong_units,
            wrong_dimensions,
            malformed,
            truncated,
            oversized,
            invalid_crop,
            no_frame,
            empty_frame,
        ] {
            let capability = ScriptedCapture {
                batch,
                ..Default::default()
            };
            assert!(matches!(
                capture_media(&capability, context(), plan(), limits()),
                Err(PlatformError::InvalidResponse(_))
            ));
        }
    }

    #[test]
    fn png_header_pixel_and_aggregate_byte_limits_are_checked_before_publication() {
        use image::ImageEncoder;
        let mut oversized_header = Vec::new();
        image::codecs::png::PngEncoder::new(&mut oversized_header)
            .write_image(&[0; 11 * 4], 11, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let mut invalid = batch();
        invalid.captures[0].png = oversized_header;
        let capability = ScriptedCapture {
            batch: invalid,
            ..Default::default()
        };
        assert!(matches!(
            capture_media(&capability, context(), plan(), limits()),
            Err(PlatformError::InvalidResponse(_))
        ));
        let mut native = batch();
        let first_bytes = native.captures[0].png.len();
        let mut second = native.captures[0].clone();
        second.attachment_id = "media-2".into();
        native.captures.push(second);
        let capability = ScriptedCapture {
            batch: native,
            ..Default::default()
        };
        let mut requests = plan();
        let mut second = requests.requests[0].clone();
        second.id = "media-2".into();
        requests.requests.push(second);
        assert!(matches!(
            capture_media(
                &capability,
                context(),
                requests,
                ImageCaptureLimits {
                    max_total_bytes: (first_bytes * 2 - 1) as u32,
                    ..limits()
                }
            ),
            Err(PlatformError::InvalidResponse(_))
        ));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0; 4], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(validate_png(
            &png,
            domain::geometry::PixelExtent {
                width: 1,
                height: 1
            },
            ImageCaptureLimits {
                max_long_edge: 1,
                max_pixels: 1,
                ..limits()
            }
        )
        .is_ok());
    }

    #[test]
    fn missing_tree_units_omit_images_without_invoking_native_but_frame_fallback_needs_no_desktop_mapping(
    ) {
        let capability = ScriptedCapture::default();
        let mut observation_missing = context();
        observation_missing.desktop_observation = None;
        let omitted = capture_media(&capability, observation_missing, plan(), limits()).unwrap();
        assert!(capability.calls.lock().unwrap().is_empty());
        assert_eq!(
            omitted.omissions[0].reason,
            LensMediaOmissionReason::GeometryUnavailable
        );
        let mut native = batch();
        native.frame.as_mut().unwrap().desktop_mapping =
            port_platform::geometry::DesktopMapping::Unavailable {
                reason: port_platform::geometry::GeometryUnavailableReason::UnverifiedConfiguration,
            };
        let capability = ScriptedCapture {
            batch: native,
            ..Default::default()
        };
        let mut fallback = plan();
        fallback.requests[0].scope = LensMediaScope::WindowFallback;
        fallback.requests[0].bounds = None;
        let mut context = context();
        context.desktop_observation = None;
        let result = capture_media(&capability, context, fallback, limits()).unwrap();
        assert_eq!(result.attachments.len(), 1);
        assert!(
            matches!(&result.attachments[0].geometry, LensMediaGeometry::Acquired { frame, .. }
            if matches!(frame.desktop_mapping, domain::geometry::DesktopMapping::Unavailable { .. }))
        );
    }
}
