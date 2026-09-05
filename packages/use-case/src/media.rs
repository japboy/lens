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
                LensMediaScope::AxElementRegion => CaptureScope::AxElementRegion,
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
            target: CaptureTarget::Registered {
                operation_id: Uuid::from_u128(11),
                window_id: 17,
            },
            target_id: "window-17".into(),
            context_id: Uuid::from_u128(42),
            context_revision: NonZeroU64::new(7).unwrap(),
        }
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
        let bounds = capture::Bounds {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        CaptureBatch {
            window_bounds: Some(bounds),
            captures: vec![CapturedImage {
                attachment_id: "media-1".into(),
                source_bounds: bounds,
                captured_bounds: bounds,
                coverage: CaptureCoverage::FullRegion,
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
                    scope: CaptureScope::AxElementRegion,
                    bounds: Some(to_platform_bounds(plan().requests[0].bounds.unwrap())),
                }]
            )]
        );
        assert_eq!(result.observed_window_frame, plan().requests[0].bounds);
        assert_eq!(result.diagnostics, ["capture diagnostic"]);
        assert_eq!(
            serde_json::to_value(&result.attachments).unwrap(),
            serde_json::json!([{
                "id":"media-1", "target_id":"window-17",
                "uri":"lens://context/00000000-0000-0000-0000-00000000002a/7/media/media-1",
                "scope":"ax_element_region", "source_node_id":"node-1",
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
