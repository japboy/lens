use crate::platform::{
    bounds_from_platform as to_domain_bounds, bounds_to_platform as to_platform_bounds,
};
use base64::prelude::*;
use domain::lens::{
    LensCoordinateSpace, LensMediaAttachment, LensMediaCapture, LensMediaCoverage,
    LensMediaOmission, LensMediaOmissionReason, LensMediaPayload, LensMediaPlan, LensMediaScope,
};
use port_platform::{
    capture::{
        Capture, CaptureBatch, CaptureCoverage, CaptureOmissionReason, CaptureRequest,
        CaptureScope, CaptureTarget,
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
    pub geometry: port_platform::geometry::CaptureGeometryExpectation,
    pub target_id: String,
    pub context_id: Uuid,
    pub context_revision: NonZeroU64,
}

pub fn capture_media(
    capability: &dyn Capture,
    context: MediaCaptureContext,
    plan: LensMediaPlan,
    limits: ImageCaptureLimits,
) -> Result<LensMediaCapture, PlatformError> {
    if matches!(context.target, CaptureTarget::Legacy { .. })
        && !matches!(
            context.geometry,
            port_platform::geometry::CaptureGeometryExpectation::ObserveCurrent
        )
    {
        return Err(PlatformError::Operation(
            "legacy capture cannot claim registered extraction geometry".into(),
        ));
    }
    if let CaptureTarget::Registered { read } = context.target {
        context
            .geometry
            .validate_for(read)
            .map_err(|error| PlatformError::Operation(error.to_string()))?;
        if read.target.operation_id.is_nil() {
            return Err(PlatformError::Operation(
                "capture operation must not be nil".into(),
            ));
        }
    }
    if plan.requests.is_empty() {
        return Ok(LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        });
    }
    let requests = plan
        .requests
        .iter()
        .map(|request| CaptureRequest {
            id: request.id.clone(),
            scope: match request.scope {
                LensMediaScope::AccessibilityElementRegion => {
                    CaptureScope::AccessibilityElementRegion
                }
                LensMediaScope::WindowFallback => CaptureScope::WindowFallback,
            },
            bounds: request.bounds.map(to_platform_bounds),
        })
        .collect::<Vec<_>>();
    if requests
        .iter()
        .map(|request| &request.id)
        .collect::<BTreeSet<_>>()
        .len()
        != requests.len()
    {
        return Err(PlatformError::Operation(
            "image capture request identities must be unique".into(),
        ));
    }
    let batch = capability.capture(context.target, &requests, limits)?;
    assemble_media(context, plan, batch)
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
    }
}

fn assemble_media(
    context: MediaCaptureContext,
    plan: LensMediaPlan,
    batch: CaptureBatch,
) -> Result<LensMediaCapture, PlatformError> {
    let expected_authority = match context.target {
        CaptureTarget::Registered { read } => Some(read),
        CaptureTarget::Legacy { .. } => None,
    };
    if batch.read != expected_authority {
        return Err(PlatformError::InvalidResponse(
            "capture returned mismatched target read".into(),
        ));
    }
    let capture_key = match (expected_authority, batch.capture, batch.captures.is_empty()) {
        (Some(read), Some(key), _) if key.read == read && !key.capture_id.is_nil() => Some(key),
        (Some(_), _, false) => {
            return Err(PlatformError::InvalidResponse(
                "capture identity is missing or disagrees with read".into(),
            ))
        }
        (None, Some(_), _) => {
            return Err(PlatformError::InvalidResponse(
                "legacy capture claimed registered pixel identity".into(),
            ))
        }
        _ => None,
    };
    let desktop_geometry = batch.geometry.clone().map(crate::geometry::descriptor);
    if !batch.captures.is_empty() {
        if let port_platform::geometry::CaptureGeometryExpectation::MatchExtraction { descriptor } =
            &context.geometry
        {
            if batch.geometry.as_ref() != Some(descriptor.as_ref()) {
                return Err(PlatformError::InvalidResponse(
                    "capture geometry does not match extraction descriptor".into(),
                ));
            }
        }
    }
    let requests_by_id = plan
        .requests
        .iter()
        .map(|request| (request.id.as_str(), request))
        .collect::<BTreeMap<_, _>>();
    let mut result = LensMediaCapture {
        observed_window_frame: batch.window_bounds.map(to_domain_bounds),
        omissions: plan.omissions,
        diagnostics: batch.diagnostics,
        ..LensMediaCapture::default()
    };
    let mut resolved = BTreeSet::new();
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
        let uri = media_uri(
            context.context_id,
            context.context_revision.get(),
            &capture.attachment_id,
        );
        result.attachments.push(LensMediaAttachment {
            geometry: capture_key
                .map(|key| {
                    crate::geometry::attachment(
                        key,
                        capture.attachment_id.clone(),
                        capture.pixel_geometry,
                    )
                })
                .transpose()
                .map_err(|error| PlatformError::InvalidResponse(error.to_string()))?,
            desktop_geometry: desktop_geometry.clone(),
            id: capture.attachment_id.clone(),
            target_id: context.target_id.clone(),
            uri: uri.clone(),
            scope: request.scope,
            source_node_id: request.source_node_id.clone(),
            source_bounds: to_domain_bounds(capture.source_bounds),
            captured_bounds: to_domain_bounds(capture.captured_bounds),
            coverage: to_domain_coverage(capture.coverage),
            coordinate_space: LensCoordinateSpace::ScreenPoints,
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
            limits: ImageCaptureLimits,
        ) -> Result<CaptureBatch, PlatformError> {
            assert_eq!(limits.max_total_bytes, 16);
            self.calls.lock().unwrap().push((target, requests.to_vec()));
            match &self.failure {
                Some(message) => Err(PlatformError::Operation(message.clone())),
                None => Ok(self.batch.clone()),
            }
        }
    }

    fn context() -> MediaCaptureContext {
        MediaCaptureContext {
            geometry: port_platform::geometry::CaptureGeometryExpectation::ObserveCurrent,
            target: CaptureTarget::Registered {
                read: port_platform::authority::TargetReadKey {
                    target: port_platform::authority::TargetAuthority {
                        operation_id: Uuid::from_u128(11),
                        receipt: Uuid::from_u128(17).try_into().unwrap(),
                    },
                    sequence: port_platform::authority::ReadSequence::FIRST,
                },
            },
            target_id: "window-17".into(),
            context_id: Uuid::from_u128(42),
            context_revision: NonZeroU64::new(7).unwrap(),
        }
    }

    #[test]
    fn capture_must_match_the_complete_extraction_descriptor() {
        use port_platform::geometry::{
            AxisAlignedTransform, CaptureGeometryExpectation, CoordinateFrame,
            ReadGeometryDescriptor, Rect, TaggedRect,
        };
        let CaptureTarget::Registered { read } = context().target else {
            unreachable!()
        };
        let descriptor = ReadGeometryDescriptor {
            read,
            window: TaggedRect {
                read,
                frame: CoordinateFrame::MacosDesktopPoints,
                rect: Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
            },
            desktop_to_target: AxisAlignedTransform {
                read,
                source: CoordinateFrame::MacosDesktopPoints,
                destination: CoordinateFrame::TargetLogical {
                    target: read.target,
                },
                scale_x: 1.0,
                scale_y: 1.0,
                translate_x: -1.0,
                translate_y: -2.0,
            },
        };
        descriptor.validate().unwrap();
        let expected_context = || {
            let mut value = context();
            value.geometry = CaptureGeometryExpectation::MatchExtraction {
                descriptor: Box::new(descriptor.clone()),
            };
            value
        };
        assert!(assemble_media(expected_context(), plan(), batch()).is_err());
        let mut matching = batch();
        matching.geometry = Some(descriptor.clone());
        assert!(assemble_media(expected_context(), plan(), matching.clone()).is_ok());
        let changed = matching.geometry.as_mut().unwrap();
        changed.desktop_to_target.scale_x = 2.0;
        changed.desktop_to_target.translate_x = -2.0;
        changed.validate().unwrap();
        assert!(assemble_media(expected_context(), plan(), matching).is_err());
    }

    fn limits() -> ImageCaptureLimits {
        ImageCaptureLimits {
            max_long_edge: 10,
            max_pixels: 100,
            max_attachment_bytes: 8,
            max_total_bytes: 16,
        }
    }

    fn plan() -> LensMediaPlan {
        LensMediaPlan {
            requests: vec![LensMediaRequest {
                id: "media-1".into(),
                scope: LensMediaScope::AccessibilityElementRegion,
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
        let bounds = capture::Bounds {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        CaptureBatch {
            capture: Some(port_platform::geometry::CaptureKey {
                read: match context().target {
                    CaptureTarget::Registered { read } => read,
                    _ => unreachable!(),
                },
                capture_id: Uuid::from_u128(20),
            }),
            geometry: None,
            read: match context().target {
                CaptureTarget::Registered { read } => Some(read),
                _ => unreachable!(),
            },
            window_bounds: Some(bounds),
            captures: vec![CapturedImage {
                attachment_id: "media-1".into(),
                source_bounds: bounds,
                captured_bounds: bounds,
                coverage: CaptureCoverage::FullRegion,
                pixel_geometry: port_platform::geometry::CapturedPixelGeometry {
                    original_extent: port_platform::geometry::PixelExtent {
                        width: 3,
                        height: 4,
                    },
                    crop: port_platform::geometry::PixelCrop {
                        x: 0,
                        y: 0,
                        extent: port_platform::geometry::PixelExtent {
                            width: 3,
                            height: 4,
                        },
                    },
                    encoded_extent: port_platform::geometry::PixelExtent {
                        width: 3,
                        height: 4,
                    },
                },
                pixel_width: 3,
                pixel_height: 4,
                // Transfer fixture: this test does not decode image pixels.
                png: vec![1, 2, 3],
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
                    scope: CaptureScope::AccessibilityElementRegion,
                    bounds: Some(to_platform_bounds(plan().requests[0].bounds.unwrap())),
                }]
            )]
        );
        assert_eq!(result.observed_window_frame, plan().requests[0].bounds);
        assert_eq!(result.diagnostics, ["capture diagnostic"]);
        let geometry = result.attachments[0].geometry.as_ref().unwrap();
        geometry.validate().unwrap();
        assert_eq!(geometry.attachment_id, "media-1");
        assert_eq!(geometry.capture.capture_id, Uuid::from_u128(20));
        assert_eq!(
            serde_json::to_value(geometry.capture.read).unwrap(),
            serde_json::to_value(batch().read.unwrap()).unwrap()
        );
        let pixels = batch().captures[0].pixel_geometry;
        assert_eq!(
            serde_json::to_value(geometry.original_extent).unwrap(),
            serde_json::to_value(pixels.original_extent).unwrap()
        );
        assert_eq!(
            serde_json::to_value(geometry.crop).unwrap(),
            serde_json::to_value(pixels.crop).unwrap()
        );
        assert_eq!(
            serde_json::to_value(geometry.encoded_extent).unwrap(),
            serde_json::to_value(pixels.encoded_extent).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&result.attachments).unwrap(),
            serde_json::json!([{
                "geometry":geometry,
                "id":"media-1", "target_id":"window-17",
                "uri":"lens://context/00000000-0000-0000-0000-00000000002a/7/media/media-1",
                "scope":"accessibility_element_region", "source_node_id":"node-1",
                "source_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "captured_bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},
                "coverage":"full_region", "coordinate_space":"screen_points",
                "mime_type":"image/png", "pixel_width":3, "pixel_height":4, "encoded_bytes":3
            }])
        );
        assert_eq!(
            result.payloads,
            [LensMediaPayload {
                attachment_id: "media-1".into(),
                uri: result.attachments[0].uri.clone(),
                mime_type: "image/png".into(),
                data: "AQID".into(),
            }]
        );
        assert!(result.omissions.is_empty());
    }

    #[test]
    fn registered_pixels_require_exact_non_nil_capture_identity() {
        for axis in 0..3 {
            let mut pixels = batch();
            match axis {
                0 => pixels.capture = None,
                1 => pixels.capture.as_mut().unwrap().capture_id = Uuid::nil(),
                2 => {
                    let key = pixels.capture.as_mut().unwrap();
                    key.read.sequence = key.read.sequence.checked_next().unwrap();
                }
                _ => unreachable!(),
            }
            assert!(assemble_media(context(), plan(), pixels).is_err());
        }
    }

    #[test]
    fn capture_rejects_missing_other_operation_and_other_receipt_authority() {
        let expected = batch().read.unwrap();
        for read in [
            None,
            Some(port_platform::authority::TargetReadKey {
                target: port_platform::authority::TargetAuthority {
                    operation_id: Uuid::from_u128(99),
                    ..expected.target
                },
                ..expected
            }),
            Some(port_platform::authority::TargetReadKey {
                target: port_platform::authority::TargetAuthority {
                    receipt: Uuid::from_u128(99).try_into().unwrap(),
                    ..expected.target
                },
                ..expected
            }),
            Some(port_platform::authority::TargetReadKey {
                sequence: expected.sequence.checked_next().unwrap(),
                ..expected
            }),
        ] {
            let capability = ScriptedCapture {
                batch: CaptureBatch { read, ..batch() },
                ..Default::default()
            };
            assert!(matches!(
                capture_media(&capability, context(), plan(), limits()),
                Err(PlatformError::InvalidResponse(_))
            ));
        }
        let capability = ScriptedCapture {
            batch: batch(),
            ..Default::default()
        };
        let mut legacy = context();
        legacy.target = CaptureTarget::Legacy { window_id: 17 };
        assert!(matches!(
            capture_media(&capability, legacy, plan(), limits()),
            Err(PlatformError::InvalidResponse(_))
        ));
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
                read: batch().read,
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
                    bounds: None,
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
}
