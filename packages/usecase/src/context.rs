//! Ordered fixed-target extraction and context assembly over injected platform capabilities.
use crate::{
    media::{capture_media, MediaCaptureContext},
    platform as conversion,
};
use domain::lens::{
    LensContext, LensMediaCapture, LensMediaOmission, LensMediaOmissionReason, LensMediaPayload,
    LensMediaPlan, LensTarget, LensTargetCapture, LensTargetSet, MAX_AX_RESOURCE_REFERENCES,
    MAX_AX_RESOURCE_URI_BYTES, MAX_AX_TOTAL_RESOURCE_URI_BYTES, MAX_LENS_CONTEXT_NODES,
    MAX_LENS_CONTEXT_TEXT_BYTES, MAX_LENS_MEDIA_ATTACHMENTS, MAX_LENS_MEDIA_ATTACHMENT_BYTES,
    MAX_LENS_MEDIA_LONG_EDGE, MAX_LENS_MEDIA_PIXELS, MAX_LENS_MEDIA_TOTAL_BYTES,
    MAX_LENS_SOURCE_NODES, MAX_LENS_SOURCE_TEXT_BYTES,
};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    capture::{Capture, CaptureTarget},
    ExtractionLimits, ImageCaptureLimits, PlatformError,
};
use std::{collections::BTreeMap, future::Future, num::NonZeroU64, pin::Pin, sync::Arc};
use uuid::Uuid;

fn validate_extraction_authority(
    extraction: &port_platform::model::ExtractionResult,
    expected_read: Option<port_platform::authority::TargetReadKey>,
) -> Result<(), String> {
    if extraction.read != expected_read {
        return Err("extraction returned mismatched target read".into());
    }
    match expected_read {
        Some(_)
            if extraction.geometry.is_none()
                && (extraction.quality != port_platform::model::ExtractionQuality::Unavailable
                    || extraction.resolved_window.is_some()
                    || !extraction.nodes.is_empty()
                    || !extraction.text.is_empty()) =>
        {
            return Err("registered extraction facts require acquired geometry".into());
        }
        None if extraction.geometry.is_some() => {
            return Err("legacy extraction cannot claim registered geometry".into());
        }
        _ => {}
    }
    for node in &extraction.nodes {
        if let Some(bounds) = &node.bounds {
            bounds
                .validate_for(extraction.geometry.as_ref())
                .map_err(|error| error.to_string())?;
            if expected_read.is_some()
                && matches!(bounds, port_platform::geometry::NodeBounds::Legacy { .. })
            {
                return Err("registered extraction cannot contain legacy node bounds".into());
            }
        }
    }
    Ok(())
}

/// The host owns worker scheduling. Each AX read and capture remains a separate await:
/// dropping the orchestration future cannot enqueue the next native operation.
/// Already-started blocking work retains the host runtime's cancellation semantics.
pub trait BlockingExecutor: Sync {
    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;
}

pub struct ContextBuildRequest<'a> {
    pub operation_id: Uuid,
    pub context_revision: u64,
    pub source_revisions: BTreeMap<String, u64>,
    pub target_set: &'a LensTargetSet,
    pub access_mode: WindowAccessMode,
}

#[derive(Debug, Clone, Copy)]
struct SourceExtractionBudget {
    max_nodes: usize,
    max_text_bytes: usize,
    limits: ExtractionLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAccessMode {
    Registered,
    Legacy { window_id: u32, pid: i32 },
}

fn source_extraction_budget(
    remaining_nodes: usize,
    remaining_text_bytes: usize,
    remaining_resource_refs: usize,
    remaining_resource_uri_bytes: usize,
) -> Option<SourceExtractionBudget> {
    let max_nodes = remaining_nodes.min(MAX_LENS_SOURCE_NODES);
    if max_nodes == 0 {
        return None;
    }
    let max_text_bytes = remaining_text_bytes.min(MAX_LENS_SOURCE_TEXT_BYTES);
    Some(SourceExtractionBudget {
        max_nodes,
        max_text_bytes,
        limits: ExtractionLimits {
            max_nodes: u32::try_from(max_nodes).expect("versioned node budget fits u32"),
            max_text_bytes: u32::try_from(max_text_bytes).expect("versioned text budget fits u32"),
            max_resource_refs: u32::try_from(remaining_resource_refs)
                .expect("versioned resource-reference budget fits u32"),
            max_resource_uri_bytes: u32::try_from(MAX_AX_RESOURCE_URI_BYTES)
                .expect("versioned resource-URI limit fits u32"),
            max_total_resource_uri_bytes: u32::try_from(remaining_resource_uri_bytes)
                .expect("versioned resource-URI group budget fits u32"),
        },
    })
}

pub struct BuiltContext {
    pub target_set: LensTargetSet,
    pub context: LensContext,
    pub payloads: Vec<LensMediaPayload>,
}

pub async fn build_context(
    executor: &impl BlockingExecutor,
    issuer: &impl crate::acquisition::ReadIssuer,
    accessibility_service: Arc<dyn Accessibility>,
    capture_service: Arc<dyn Capture>,
    request: ContextBuildRequest<'_>,
) -> Result<BuiltContext, String> {
    let ContextBuildRequest {
        operation_id,
        context_revision,
        source_revisions,
        target_set,
        access_mode,
    } = request;
    let validated = LensTargetSet::try_new(
        target_set.selection_id,
        target_set
            .targets
            .iter()
            .map(LensTarget::selected_window)
            .collect(),
    )
    .map_err(|error| error.to_string())?;
    if operation_id != target_set.selection_id || !target_set.has_same_identity(&validated) {
        return Err("context target authority or ordering is invalid".into());
    }
    if matches!(access_mode, WindowAccessMode::Legacy { .. }) && target_set.targets.len() != 1 {
        return Err("legacy diagnostic reads require exactly one target".into());
    }
    let mut remaining_nodes = MAX_LENS_CONTEXT_NODES;
    let mut remaining_text_bytes = MAX_LENS_CONTEXT_TEXT_BYTES;
    let mut remaining_resource_refs = MAX_AX_RESOURCE_REFERENCES;
    let mut remaining_resource_uri_bytes = MAX_AX_TOTAL_RESOURCE_URI_BYTES;
    let mut remaining_media_attachments = MAX_LENS_MEDIA_ATTACHMENTS;
    let mut remaining_media_bytes = MAX_LENS_MEDIA_TOTAL_BYTES;
    let mut captures = Vec::with_capacity(target_set.targets.len());
    let mut payloads = Vec::new();
    let mut observed_facts = BTreeMap::new();

    for target in &target_set.targets {
        let read = match access_mode {
            WindowAccessMode::Registered => {
                let authority = port_platform::authority::TargetAuthority {
                    operation_id,
                    receipt: conversion::window_identity(&target.identity)
                        .map_err(|error| error.to_string())?
                        .receipt,
                };
                let read = issuer.reserve(authority)?;
                if read.target != authority {
                    return Err("read issuer returned mismatched target authority".into());
                }
                Some(read)
            }
            WindowAccessMode::Legacy { .. } => None,
        };
        let extraction_budget = source_extraction_budget(
            remaining_nodes,
            remaining_text_bytes,
            remaining_resource_refs,
            remaining_resource_uri_bytes,
        );
        let mut geometry = port_platform::geometry::CaptureGeometryExpectation::ObserveCurrent;
        let accessibility = if let Some(extraction_budget) = extraction_budget {
            let max_nodes = extraction_budget.max_nodes;
            let max_text_bytes = extraction_budget.max_text_bytes;
            let limits = extraction_budget.limits;
            let extraction_target = match access_mode {
                WindowAccessMode::Registered => ExtractionTarget::Registered {
                    read: read.ok_or("registered extraction requires a read key")?,
                },
                WindowAccessMode::Legacy { window_id, pid } => {
                    ExtractionTarget::Legacy(port_platform::model::LegacyWindow {
                        window_id,
                        pid,
                        facts: conversion::selected_window(&target.selected_window())
                            .map_err(|error| error.to_string())?
                            .facts,
                    })
                }
            };
            let expected_read = match &extraction_target {
                ExtractionTarget::Registered { read } => Some(*read),
                ExtractionTarget::Legacy(_) => None,
            };
            let service = accessibility_service.clone();
            let mut extraction = match executor
                .run(move || service.extract(extraction_target, limits))
                .await
            {
                Ok(Ok(extraction)) => {
                    validate_extraction_authority(&extraction, expected_read)?;
                    if let Some(descriptor) = extraction.geometry.as_ref() {
                        descriptor.validate().map_err(|error| error.to_string())?;
                        if Some(descriptor.read) != expected_read {
                            return Err("extraction geometry returned mismatched read".into());
                        }
                        geometry =
                            port_platform::geometry::CaptureGeometryExpectation::MatchExtraction {
                                descriptor: Box::new(descriptor.clone()),
                            };
                    }
                    conversion::extraction(extraction)
                }
                Ok(Err(error)) => LensContext::unavailable_accessibility(error.to_string()),
                Err(error) => LensContext::unavailable_accessibility(error.to_string()),
            };
            if extraction.metrics.truncated_nodes && max_nodes < MAX_LENS_SOURCE_NODES {
                extraction.diagnostics.push(format!(
                    "{} reached the version 1 group traversal-node budget",
                    target.id
                ));
            }
            if extraction.metrics.truncated_text && max_text_bytes < MAX_LENS_SOURCE_TEXT_BYTES {
                extraction.diagnostics.push(format!(
                    "{} reached the version 1 group text-byte budget",
                    target.id
                ));
            }
            remaining_nodes = remaining_nodes.saturating_sub(extraction.metrics.visited_nodes);
            remaining_text_bytes =
                remaining_text_bytes.saturating_sub(extraction.metrics.text_bytes);
            remaining_resource_refs =
                remaining_resource_refs.saturating_sub(extraction.metrics.resource_ref_count);
            remaining_resource_uri_bytes =
                remaining_resource_uri_bytes.saturating_sub(extraction.metrics.resource_uri_bytes);
            extraction
        } else {
            LensContext::unavailable_accessibility(format!(
                "{} was not extracted because the version 1 group traversal-node budget was exhausted",
                target.id
            ))
        };

        let mut current_target = target.clone();
        if let Some(resolved_window) = &accessibility.resolved_window {
            current_target.facts = resolved_window.facts.clone();
            observed_facts.insert(target.id.clone(), current_target.facts.clone());
        }
        let plan = LensMediaPlan::from_accessibility(
            &current_target,
            &accessibility,
            remaining_media_attachments,
        );
        let mut media = capture_target_media(
            executor,
            capture_service.clone(),
            TargetCaptureRequest {
                operation_id,
                read,
                context_revision,
                target: current_target.clone(),
                plan: plan.clone(),
                geometry,
                remaining_total_bytes: remaining_media_bytes,
                access_mode,
            },
        )
        .await;
        if accessibility.resolved_window.is_some()
            && !media.attachments.is_empty()
            && media.observed_window_frame != Some(current_target.facts.frame)
        {
            media = failed_media_capture(
                target.id.clone(),
                plan,
                "Extraction and capture window frames differ; captured media is unavailable."
                    .into(),
            );
        }
        if accessibility.resolved_window.is_none() {
            if let Some(observed_frame) = media.observed_window_frame {
                current_target.facts.frame = observed_frame;
                observed_facts.insert(target.id.clone(), current_target.facts.clone());
            }
        }
        remaining_media_attachments =
            remaining_media_attachments.saturating_sub(media.attachments.len());
        remaining_media_bytes = remaining_media_bytes.saturating_sub(
            media
                .attachments
                .iter()
                .map(|attachment| attachment.encoded_bytes)
                .sum(),
        );
        payloads.append(&mut media.payloads);
        captures.push(LensTargetCapture {
            accessibility,
            media,
        });
    }

    let refreshed_target_set = target_set
        .refresh_observable_facts(context_revision, observed_facts)
        .map_err(|error| error.to_string())?;
    let context = LensContext::from_captures_at_revision(
        operation_id,
        context_revision,
        source_revisions,
        &refreshed_target_set,
        captures,
    )
    .map_err(|error| error.to_string())?;
    Ok(BuiltContext {
        target_set: refreshed_target_set,
        context,
        payloads,
    })
}

struct TargetCaptureRequest {
    geometry: port_platform::geometry::CaptureGeometryExpectation,
    operation_id: Uuid,
    read: Option<port_platform::authority::TargetReadKey>,
    context_revision: u64,
    target: LensTarget,
    plan: LensMediaPlan,
    remaining_total_bytes: usize,
    access_mode: WindowAccessMode,
}

async fn capture_target_media(
    executor: &impl BlockingExecutor,
    capture_service: Arc<dyn Capture>,
    request: TargetCaptureRequest,
) -> LensMediaCapture {
    let TargetCaptureRequest {
        geometry,
        operation_id,
        read,
        context_revision,
        target,
        plan,
        remaining_total_bytes,
        access_mode,
    } = request;
    if plan.requests.is_empty() {
        return LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        };
    }
    if remaining_total_bytes == 0 {
        return byte_budget_media_capture(target.id, plan);
    }

    let target_id = target.id.clone();
    let capture_plan = plan.clone();
    let limits = ImageCaptureLimits {
        max_long_edge: u32::try_from(MAX_LENS_MEDIA_LONG_EDGE)
            .expect("versioned media long-edge limit fits u32"),
        max_pixels: u32::try_from(MAX_LENS_MEDIA_PIXELS)
            .expect("versioned media pixel limit fits u32"),
        max_attachment_bytes: u32::try_from(MAX_LENS_MEDIA_ATTACHMENT_BYTES)
            .expect("versioned media attachment-byte limit fits u32"),
        max_total_bytes: u32::try_from(remaining_total_bytes)
            .expect("versioned media total-byte budget fits u32"),
    };
    let mut result = match executor
        .run(move || {
            let (capture_target, capture_revision) = match access_mode {
                WindowAccessMode::Registered => (
                    CaptureTarget::Registered {
                        read: read.ok_or_else(|| {
                            PlatformError::Operation(
                                "registered capture requires a read key".into(),
                            )
                        })?,
                    },
                    NonZeroU64::new(context_revision).ok_or_else(|| {
                        PlatformError::Operation(
                            "registered image capture context revision must be non-zero".into(),
                        )
                    })?,
                ),
                // Preserve the explicit legacy validation path and its revision-one capture namespace.
                WindowAccessMode::Legacy { window_id, .. } => {
                    (CaptureTarget::Legacy { window_id }, NonZeroU64::MIN)
                }
            };
            capture_media(
                capture_service.as_ref(),
                MediaCaptureContext {
                    geometry,
                    target: capture_target,
                    target_id: target.id,
                    context_id: operation_id,
                    context_revision: capture_revision,
                },
                capture_plan,
                limits,
            )
        })
        .await
    {
        Ok(Ok(capture)) => capture,
        Ok(Err(error)) => failed_media_capture(target_id, plan, error.to_string()),
        Err(error) => failed_media_capture(target_id, plan, error.to_string()),
    };
    if context_revision != 1 {
        let prefix = format!("lens://context/{operation_id}/1/media/");
        let replacement = format!("lens://context/{operation_id}/{context_revision}/media/");
        for attachment in &mut result.attachments {
            attachment.uri = attachment.uri.replacen(&prefix, &replacement, 1);
        }
        for payload in &mut result.payloads {
            payload.uri = payload.uri.replacen(&prefix, &replacement, 1);
        }
    }
    result
}

fn byte_budget_media_capture(target_id: String, plan: LensMediaPlan) -> LensMediaCapture {
    let mut omissions = plan.omissions;
    omissions.extend(plan.requests.into_iter().map(|request| LensMediaOmission {
        target_id: target_id.clone(),
        attachment_id: Some(request.id),
        source_node_id: request.source_node_id,
        reason: LensMediaOmissionReason::ByteBudget,
        omitted_count: 1,
        first_order: None,
        last_order: None,
        detail:
            "The versioned 16 MiB media group byte budget was exhausted before this target.".into(),
    }));
    LensMediaCapture {
        omissions,
        ..LensMediaCapture::default()
    }
}

fn failed_media_capture(
    target_id: String,
    plan: LensMediaPlan,
    message: String,
) -> LensMediaCapture {
    let mut omissions = plan.omissions;
    omissions.extend(plan.requests.into_iter().map(|request| LensMediaOmission {
        target_id: target_id.clone(),
        attachment_id: Some(request.id),
        source_node_id: request.source_node_id,
        reason: LensMediaOmissionReason::CaptureFailed,
        omitted_count: 1,
        first_order: None,
        last_order: None,
        detail: message.clone(),
    }));
    LensMediaCapture {
        omissions,
        diagnostics: vec![format!("ScreenCaptureKit media capture failed: {message}")],
        ..LensMediaCapture::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_facts_require_geometry_but_empty_unavailable_does_not() {
        let read = port_platform::authority::TargetReadKey {
            target: port_platform::authority::TargetAuthority {
                operation_id: Uuid::from_u128(1),
                receipt: Uuid::from_u128(2).try_into().unwrap(),
            },
            sequence: port_platform::authority::ReadSequence::FIRST,
        };
        let empty = serde_json::json!({"read":read,"geometry":null,"quality":"unavailable","nodes":[],"text":"","diagnostics":[]});
        let decode = |value| {
            serde_json::from_value::<port_platform::model::ExtractionResult>(value).unwrap()
        };
        assert!(validate_extraction_authority(&decode(empty.clone()), Some(read)).is_ok());
        for (field, value) in [
            ("quality", serde_json::json!("full")),
            ("quality", serde_json::json!("partial")),
            ("text", serde_json::json!("still useful")),
            (
                "nodes",
                serde_json::json!([{"id":"n","order":0,"depth":0,"source_api":"macos_ax","semantic_kind":"text","node_purpose":"content","children":[]}]),
            ),
            (
                "resolved_window",
                serde_json::json!({"facts":{"application_name":"Fixture","application_id":"fixture","title":"Fixture","frame":{"x":0.0,"y":0.0,"width":100.0,"height":80.0}},"resolution_score":1.0}),
            ),
        ] {
            let mut wire = empty.clone();
            wire[field] = value;
            assert!(
                validate_extraction_authority(&decode(wire), Some(read)).is_err(),
                "{field}"
            );
        }
        let mut other = read;
        other.sequence = read.sequence.checked_next().unwrap();
        assert!(validate_extraction_authority(&decode(empty.clone()), Some(other)).is_err());
        assert!(validate_extraction_authority(&decode(empty.clone()), None).is_err());
        let mut legacy = empty;
        legacy["read"] = serde_json::Value::Null;
        assert!(validate_extraction_authority(&decode(legacy.clone()), None).is_ok());
        assert!(validate_extraction_authority(&decode(legacy.clone()), Some(read)).is_err());
        legacy["geometry"] = serde_json::json!({
            "read":read,
            "window":{"read":read,"frame":{"kind":"macos_desktop_points"},"rect":{"x":0.0,"y":0.0,"width":100.0,"height":80.0}},
            "desktop_to_target":{"read":read,"source":{"kind":"macos_desktop_points"},"destination":{"kind":"target_logical","target":read.target},"scale_x":1.0,"scale_y":1.0,"translate_x":0.0,"translate_y":0.0}
        });
        assert!(validate_extraction_authority(&decode(legacy.clone()), None).is_err());
        legacy["read"] = serde_json::to_value(read).unwrap();
        legacy["quality"] = serde_json::json!("full");
        assert!(validate_extraction_authority(&decode(legacy), Some(read)).is_ok());
    }
    use domain::{lens::LensInput, model::SelectedWindow};
    use port_platform::{
        capture::{CaptureBatch, CaptureCoverage, CaptureRequest, CaptureScope, CapturedImage},
        model::{self as facts, ExtractionResult},
    };
    use serde_json::json;
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
        task::{Context, Poll, Waker},
    };

    #[derive(Debug)]
    enum Call {
        Accessibility(ExtractionTarget, ExtractionLimits),
        Capture(CaptureTarget, Vec<CaptureRequest>, ImageCaptureLimits),
    }

    #[derive(Default)]
    struct Sources {
        reads: Mutex<Option<crate::acquisition::OperationReadState>>,
        calls: Mutex<Vec<Call>>,
        extractions: Mutex<VecDeque<Result<ExtractionResult, String>>>,
        capture_error: Option<String>,
        capture_frame_override: Option<facts::Bounds>,
        extraction_read_override: Option<Option<port_platform::authority::TargetReadKey>>,
    }

    impl crate::acquisition::ReadIssuer for Sources {
        fn reserve(
            &self,
            target: port_platform::authority::TargetAuthority,
        ) -> Result<port_platform::authority::TargetReadKey, String> {
            let mut reads = self.reads.lock().unwrap();
            let state = reads.get_or_insert_with(|| {
                crate::acquisition::OperationReadState::new(target.operation_id).unwrap()
            });
            state.reserve(target).map_err(|error| format!("{error:?}"))
        }
    }

    impl Accessibility for Sources {
        fn extract(
            &self,
            target: ExtractionTarget,
            limits: ExtractionLimits,
        ) -> Result<ExtractionResult, PlatformError> {
            let read = match &target {
                ExtractionTarget::Registered { read } => Some(*read),
                ExtractionTarget::Legacy(_) => None,
            };
            self.calls
                .lock()
                .unwrap()
                .push(Call::Accessibility(target, limits));
            self.extractions
                .lock()
                .unwrap()
                .pop_front()
                .expect("declared source result")
                .map(|mut result| {
                    result.read = self.extraction_read_override.unwrap_or(read);
                    result.geometry = result.read.and_then(|read| {
                        result.resolved_window.as_ref().map(|window| {
                            use port_platform::geometry::*;
                            let frame = window.facts.frame;
                            ReadGeometryDescriptor {
                                read,
                                window: TaggedRect {
                                    read,
                                    frame: CoordinateFrame::MacosDesktopPoints,
                                    rect: Rect {
                                        x: frame.x,
                                        y: frame.y,
                                        width: frame.width,
                                        height: frame.height,
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
                                    translate_x: -frame.x,
                                    translate_y: -frame.y,
                                },
                            }
                        })
                    });
                    for node in &mut result.nodes {
                        if let Some(port_platform::geometry::NodeBounds::Legacy { geometry }) =
                            &node.bounds
                        {
                            if let Some(descriptor) = &result.geometry {
                                node.bounds =
                                    Some(port_platform::geometry::NodeBounds::Registered {
                                        geometry: port_platform::geometry::TaggedRect {
                                            read: descriptor.read,
                                            frame: descriptor.window.frame.clone(),
                                            rect: geometry.rect,
                                        },
                                    });
                            }
                        }
                    }
                    result
                })
                .map_err(PlatformError::Operation)
        }
    }

    impl Capture for Sources {
        fn capture(
            &self,
            target: CaptureTarget,
            requests: &[CaptureRequest],
            limits: ImageCaptureLimits,
        ) -> Result<CaptureBatch, PlatformError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Capture(target, requests.to_vec(), limits));
            if let Some(message) = &self.capture_error {
                return Err(PlatformError::Operation(message.clone()));
            }
            let bounds = self.capture_frame_override.unwrap_or(facts::Bounds {
                x: 10.0,
                y: 20.0,
                width: 200.0,
                height: 100.0,
            });
            Ok(CaptureBatch {
                geometry: match target {
                    CaptureTarget::Registered { read } => {
                        Some(port_platform::geometry::ReadGeometryDescriptor {
                            read,
                            window: port_platform::geometry::TaggedRect {
                                read,
                                frame: port_platform::geometry::CoordinateFrame::MacosDesktopPoints,
                                rect: port_platform::geometry::Rect {
                                    x: bounds.x,
                                    y: bounds.y,
                                    width: bounds.width,
                                    height: bounds.height,
                                },
                            },
                            desktop_to_target: port_platform::geometry::AxisAlignedTransform {
                                read,
                                source:
                                    port_platform::geometry::CoordinateFrame::MacosDesktopPoints,
                                destination:
                                    port_platform::geometry::CoordinateFrame::TargetLogical {
                                        target: read.target,
                                    },
                                scale_x: 1.0,
                                scale_y: 1.0,
                                translate_x: -bounds.x,
                                translate_y: -bounds.y,
                            },
                        })
                    }
                    CaptureTarget::Legacy { .. } => None,
                },
                capture: match target {
                    CaptureTarget::Registered { read } => {
                        Some(port_platform::geometry::CaptureKey {
                            read,
                            capture_id: Uuid::from_u128(20),
                        })
                    }
                    CaptureTarget::Legacy { .. } => None,
                },
                read: match target {
                    CaptureTarget::Registered { read } => Some(read),
                    CaptureTarget::Legacy { .. } => None,
                },
                window_bounds: Some(bounds),
                captures: requests
                    .iter()
                    .map(|request| {
                        let region = request.bounds.unwrap_or(bounds);
                        // This fixture explicitly models one original pixel per desktop point,
                        // then resizes the selected integer crop to its existing 1x1 payload.
                        let pixel = |value: f64| {
                            assert!(
                                value.is_finite()
                                    && value >= 0.0
                                    && value <= f64::from(u32::MAX)
                                    && value.fract() == 0.0
                            );
                            value as u32
                        };
                        CapturedImage {
                            attachment_id: request.id.clone(),
                            source_bounds: request.bounds.unwrap_or(bounds),
                            captured_bounds: request.bounds.unwrap_or(bounds),
                            coverage: CaptureCoverage::FullRegion,
                            pixel_geometry: port_platform::geometry::CapturedPixelGeometry {
                                original_extent: port_platform::geometry::PixelExtent {
                                    width: pixel(bounds.width),
                                    height: pixel(bounds.height),
                                },
                                crop: port_platform::geometry::PixelCrop {
                                    x: pixel(region.x - bounds.x),
                                    y: pixel(region.y - bounds.y),
                                    extent: port_platform::geometry::PixelExtent {
                                        width: pixel(region.width),
                                        height: pixel(region.height),
                                    },
                                },
                                encoded_extent: port_platform::geometry::PixelExtent {
                                    width: 1,
                                    height: 1,
                                },
                            },
                            pixel_width: 1,
                            pixel_height: 1,
                            // Opaque decoded payload: PNG validation belongs to the adapter, not this test service.
                            png: vec![137, 80, 78, 71],
                        }
                    })
                    .collect(),
                ..CaptureBatch::default()
            })
        }
    }

    #[derive(Default)]
    struct Executor {
        calls: AtomicUsize,
        fail_at: Option<usize>,
        pending_after: Option<usize>,
    }

    impl BlockingExecutor for Executor {
        fn run<T: Send + 'static>(
            &self,
            work: impl FnOnce() -> T + Send + 'static,
        ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_at == Some(call) {
                return Box::pin(std::future::ready(Err("worker failed".into())));
            }
            let result = work();
            if self.pending_after == Some(call) {
                // Started work can finish even if the orchestration is dropped before its join.
                Box::pin(async move {
                    std::future::pending::<()>().await;
                    Ok(result)
                })
            } else {
                Box::pin(std::future::ready(Ok(result)))
            }
        }
    }

    fn ready<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("immediate executor must finish without an external runtime"),
        }
    }

    fn targets(count: u32) -> LensTargetSet {
        let windows = (1..=count)
            .rev()
            .map(|id| {
                serde_json::from_value::<SelectedWindow>(json!({
                    "operation_id":Uuid::from_u128(7), "receipt":Uuid::from_u128(u128::from(id)), "selection_ordinal":id,"application_id":"com.example.context",
                    "title":"Picker title", "application_name":"Fixture",
                    "frame":{"x":0.0,"y":0.0,"width":200.0,"height":100.0}
                }))
                .unwrap()
            })
            .collect();
        LensTargetSet::try_new(Uuid::from_u128(7), windows).unwrap()
    }

    fn extraction(image: bool) -> ExtractionResult {
        let mut nodes = vec![json!({
            "id":"text", "parent_id":null, "order":0, "depth":0,
            "source_api":"macos_ax", "semantic_kind":"text", "node_purpose":"content", "native_role":"AXStaticText",
            "value":"Current text", "bounds":null, "children":[]
        })];
        if image {
            nodes.push(json!({
                "id":"image", "parent_id":null, "order":1, "depth":0,
                "source_api":"macos_ax", "semantic_kind":"image", "node_purpose":"content", "native_role":"AXImage",
                "description":"Current image",
                "bounds":{"kind":"legacy","geometry":{"frame":"macos_desktop_points","rect":{"x":30.0,"y":40.0,"width":20.0,"height":10.0}}}, "children":[]
            }));
        }
        serde_json::from_value(json!({
            "quality":"full",
            "resolved_window":{
                "facts":{"title":"Observed title","application_name":"Fixture","application_id":"com.example.context",
                    "frame":{"x":10.0,"y":20.0,"width":200.0,"height":100.0}},
                "resolution_score":1.0
            },
            "nodes":nodes, "text":"Current text",
            "metrics":{"visited_nodes":2,"text_bytes":12,"resource_ref_count":2,"resource_uri_bytes":20,
                "offscreen_text_nodes":0,"virtualization_signals":0,"truncated_nodes":false,
                "truncated_text":false,"children_read_errors":0,
                "omitted_resource_refs":0,"resource_read_errors":0},
            "diagnostics":[]
        })).unwrap()
    }

    fn request(
        targets: &LensTargetSet,
        mode: WindowAccessMode,
        revision: u64,
    ) -> ContextBuildRequest<'_> {
        ContextBuildRequest {
            operation_id: targets.selection_id,
            context_revision: revision,
            source_revisions: targets
                .targets
                .iter()
                .map(|target| (target.id.clone(), revision))
                .collect(),
            target_set: targets,
            access_mode: mode,
        }
    }

    #[test]
    fn context_build_runs_real_graph_media_and_facts_assembly_in_canonical_source_order() {
        let targets = targets(2);
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([Ok(extraction(true)), Ok(extraction(true))])),
            ..Sources::default()
        });
        let built = ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 3),
        ))
        .unwrap();
        assert!(built.target_set.has_same_identity(&targets));
        for target in &built.target_set.targets {
            assert_eq!(target.facts.title, "Observed title");
            assert_eq!(
                target.facts.frame.x, 10.0,
                "extraction frame remains attached to its nodes"
            );
            assert_eq!(target.facts_revision, 3);
        }
        assert_eq!(built.context.context_id, targets.selection_id);
        assert_eq!(built.context.revision, 3);
        assert_eq!(built.context.sources.len(), 2);
        assert!(built
            .context
            .sources
            .iter()
            .all(|source| source.revision == 3));
        assert_eq!(built.payloads.len(), 2);
        assert!(built.payloads[0]
            .uri
            .ends_with("/3/media/media-window-00000000-0000-0000-0000-000000000001-image"));
        assert!(built.payloads[1]
            .uri
            .ends_with("/3/media/media-window-00000000-0000-0000-0000-000000000002-image"));
        let input =
            LensInput::from_context(&built.context).expect("real graph/media produces an input");
        assert_eq!(input.sources.len(), 2);
        let calls = sources.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        for (index, pair) in calls.as_chunks::<2>().0.iter().enumerate() {
            let id = u32::try_from(index + 1).unwrap();
            let Call::Accessibility(ExtractionTarget::Registered { read }, limits) = &pair[0]
            else {
                panic!("registered AX precedes capture")
            };
            assert_eq!(read.target.operation_id, targets.selection_id);
            assert_eq!(
                Uuid::from(read.target.receipt),
                Uuid::from_u128(u128::from(id))
            );
            assert_eq!(limits.max_nodes, 30_000);
            assert_eq!(limits.max_resource_refs, 256 - index as u32 * 2);
            assert_eq!(
                limits.max_total_resource_uri_bytes,
                131_072 - index as u32 * 20
            );
            let Call::Capture(target, requests, limits) = &pair[1] else {
                panic!("capture follows AX")
            };
            assert_eq!(*target, CaptureTarget::Registered { read: *read });
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].scope, CaptureScope::AccessibilityElementRegion);
            assert_eq!(limits.max_total_bytes, 16_777_216 - index as u32 * 4);
        }
    }

    #[test]
    fn capture_frame_change_discards_media_without_reframing_extracted_nodes() {
        let targets = targets(1);
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([Ok(extraction(true))])),
            capture_frame_override: Some(facts::Bounds {
                x: 12.0,
                y: 24.0,
                width: 200.0,
                height: 100.0,
            }),
            ..Sources::default()
        });
        let built = ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 3),
        ))
        .unwrap();
        assert_eq!(built.target_set.targets[0].facts.frame.x, 10.0);
        assert_eq!(built.target_set.targets[0].facts.frame.y, 20.0);
        assert!(built.payloads.is_empty());
        let wire = serde_json::to_value(&built.context).unwrap();
        assert!(wire
            .to_string()
            .contains("capture geometry does not match extraction descriptor"));
        assert!(wire.to_string().contains("Current text"));
    }

    #[test]
    fn mismatched_extraction_authority_stops_before_facts_or_capture() {
        let targets = targets(1);
        let expected = port_platform::authority::TargetReadKey {
            target: port_platform::authority::TargetAuthority {
                operation_id: targets.selection_id,
                receipt: targets.targets[0].identity.receipt.try_into().unwrap(),
            },
            sequence: port_platform::authority::ReadSequence::FIRST,
        };
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
            let sources = Arc::new(Sources {
                extractions: Mutex::new(VecDeque::from([Ok(extraction(true))])),
                extraction_read_override: Some(read),
                ..Sources::default()
            });
            let result = ready(build_context(
                &Executor::default(),
                sources.as_ref(),
                sources.clone(),
                sources.clone(),
                request(&targets, WindowAccessMode::Registered, 1),
            ));
            assert!(matches!(result, Err(message) if message.contains("mismatched target read")));
            assert_eq!(sources.calls.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn legacy_validation_remains_explicit_and_rewrites_the_final_media_revision() {
        let targets = targets(1);
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([Ok(extraction(true))])),
            ..Sources::default()
        });
        let built = ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(
                &targets,
                WindowAccessMode::Legacy {
                    window_id: 1,
                    pid: 42,
                },
                7,
            ),
        ))
        .unwrap();
        let calls = sources.calls.lock().unwrap();
        assert!(
            matches!(&calls[0], Call::Accessibility(ExtractionTarget::Legacy(window), _) if window.window_id == 1 && window.pid == 42 && window.facts == conversion::selected_window(&targets.targets[0].selected_window()).unwrap().facts)
        );
        assert!(matches!(
            &calls[1],
            Call::Capture(CaptureTarget::Legacy { window_id: 1 }, _, _)
        ));
        assert!(built.payloads[0]
            .uri
            .ends_with("/7/media/media-window-00000000-0000-0000-0000-000000000001-image"));
        assert_eq!(built.context.media[0].uri, built.payloads[0].uri);
    }

    #[test]
    fn native_and_worker_failures_remain_explicit_with_target_local_fallback() {
        let targets = targets(1);
        for fail_at in [None, Some(1), Some(2)] {
            let sources = Arc::new(Sources {
                extractions: Mutex::new(VecDeque::from([Err("AX unavailable".into())])),
                capture_error: Some("capture unavailable".into()),
                ..Sources::default()
            });
            let executor = Executor {
                fail_at,
                ..Executor::default()
            };
            let built = ready(build_context(
                &executor,
                sources.as_ref(),
                sources.clone(),
                sources.clone(),
                request(&targets, WindowAccessMode::Registered, 1),
            ))
            .unwrap();
            assert_eq!(
                built.context.quality,
                domain::model::ExtractionQuality::Unavailable
            );
            assert!(LensInput::from_context(&built.context).is_none());
            assert!(built.payloads.is_empty());
            let wire = serde_json::to_string(&built.context).unwrap();
            assert!(wire.contains("capture_failed"));
            assert!(wire.contains(if fail_at.is_some() {
                "worker failed"
            } else {
                "AX unavailable"
            }));
            for call in sources.calls.lock().unwrap().iter() {
                if let Call::Capture(_, requests, _) = call {
                    assert_eq!(requests.len(), 1);
                    assert_eq!(requests[0].scope, CaptureScope::WindowFallback);
                }
            }
        }
    }

    #[test]
    fn group_node_exhaustion_skips_only_ax_and_preserves_later_source_fallback() {
        let targets = targets(3);
        let mut full_budget = extraction(false);
        full_budget.metrics.visited_nodes = MAX_LENS_SOURCE_NODES;
        full_budget.metrics.text_bytes = MAX_LENS_SOURCE_TEXT_BYTES;
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([Ok(full_budget.clone()), Ok(full_budget)])),
            ..Sources::default()
        });
        let built = ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 1),
        ))
        .unwrap();
        assert_eq!(built.context.sources.len(), 3);
        let calls = sources.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(matches!(calls[0], Call::Accessibility(_, _)));
        assert!(matches!(calls[1], Call::Accessibility(_, _)));
        assert!(
            matches!(&calls[2], Call::Capture(CaptureTarget::Registered {read}, requests, _) if Uuid::from(read.target.receipt) == Uuid::from_u128(3) && requests[0].scope == CaptureScope::WindowFallback)
        );
        assert!(serde_json::to_string(&built.context)
            .unwrap()
            .contains("group traversal-node budget was exhausted"));
    }

    #[test]
    fn exhausted_group_text_budget_still_reads_the_next_graph_and_image() {
        let targets = targets(3);
        let mut text_budget = extraction(false);
        text_budget.metrics.text_bytes = MAX_LENS_SOURCE_TEXT_BYTES;
        let mut final_source = extraction(true);
        final_source.metrics.text_bytes = 0;
        final_source.metrics.truncated_text = true;
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([
                Ok(text_budget.clone()),
                Ok(text_budget),
                Ok(final_source),
            ])),
            ..Sources::default()
        });
        let built = ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 1),
        ))
        .unwrap();
        let calls = sources.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        let Call::Accessibility(ExtractionTarget::Registered { read }, limits) = &calls[2] else {
            panic!("third source AX remains scheduled");
        };
        assert_eq!(Uuid::from(read.target.receipt), Uuid::from_u128(3));
        assert_eq!(limits.max_text_bytes, 0);
        assert_eq!(limits.max_nodes, MAX_LENS_SOURCE_NODES as u32);
        assert!(
            matches!(&calls[3], Call::Capture(CaptureTarget::Registered {read}, requests, _) if Uuid::from(read.target.receipt) == Uuid::from_u128(3) && requests[0].scope == CaptureScope::AccessibilityElementRegion)
        );
        assert_eq!(built.payloads.len(), 1);
        assert!(serde_json::to_string(&built.context)
            .unwrap()
            .contains("group text-byte budget"));
    }

    #[test]
    fn empty_plan_and_exhausted_media_budget_do_not_schedule_capture() {
        let targets = targets(1);
        let executor = Executor::default();
        let sources = Arc::new(Sources::default());
        for empty in [true, false] {
            let plan = if empty {
                LensMediaPlan::default()
            } else {
                LensMediaPlan::from_accessibility(
                    &targets.targets[0],
                    &LensContext::unavailable_accessibility("missing AX"),
                    1,
                )
            };
            let media = ready(capture_target_media(
                &executor,
                sources.clone(),
                TargetCaptureRequest {
                    operation_id: targets.selection_id,
                    geometry: port_platform::geometry::CaptureGeometryExpectation::ObserveCurrent,
                    read: None,
                    context_revision: 1,
                    target: targets.targets[0].clone(),
                    plan,
                    remaining_total_bytes: 0,
                    access_mode: WindowAccessMode::Registered,
                },
            ));
            assert!(media.payloads.is_empty());
            assert_eq!(media.omissions.len(), usize::from(!empty));
            if !empty {
                assert_eq!(
                    media.omissions[0].reason,
                    LensMediaOmissionReason::ByteBudget
                );
            }
        }
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        assert!(sources.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn dropping_context_build_at_each_native_await_never_enqueues_the_next_operation() {
        let targets = targets(2);
        for pending_after in 1..=4 {
            let sources = Arc::new(Sources {
                extractions: Mutex::new(VecDeque::from([
                    Ok(extraction(true)),
                    Ok(extraction(true)),
                ])),
                ..Sources::default()
            });
            let executor = Executor {
                pending_after: Some(pending_after),
                ..Executor::default()
            };
            let mut future = Box::pin(build_context(
                &executor,
                sources.as_ref(),
                sources.clone(),
                sources.clone(),
                request(&targets, WindowAccessMode::Registered, 1),
            ));
            fn assert_send<T: Send>(_: &T) {}
            assert_send(&future);
            assert!(future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending());
            assert_eq!(sources.calls.lock().unwrap().len(), pending_after);
            drop(future);
            assert_eq!(executor.calls.load(Ordering::SeqCst), pending_after);
            assert_eq!(sources.calls.lock().unwrap().len(), pending_after);
        }
    }

    #[test]
    fn cancelled_cycle_retry_at_same_context_revision_uses_a_fresh_read_key() {
        let targets = targets(1);
        let sources = Arc::new(Sources {
            extractions: Mutex::new(VecDeque::from([Ok(extraction(true)), Ok(extraction(true))])),
            ..Sources::default()
        });
        let pending = Executor {
            pending_after: Some(1),
            ..Executor::default()
        };
        let mut abandoned = Box::pin(build_context(
            &pending,
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 1),
        ));
        assert!(abandoned
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        drop(abandoned);
        ready(build_context(
            &Executor::default(),
            sources.as_ref(),
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 1),
        ))
        .unwrap();
        let calls = sources.calls.lock().unwrap();
        let Call::Accessibility(ExtractionTarget::Registered { read: first }, _) = &calls[0] else {
            panic!("first extraction")
        };
        let Call::Accessibility(ExtractionTarget::Registered { read: retry }, _) = &calls[1] else {
            panic!("retry extraction")
        };
        let Call::Capture(CaptureTarget::Registered { read: capture }, _, _) = &calls[2] else {
            panic!("retry capture")
        };
        assert_eq!(first.target, retry.target);
        assert_eq!(retry.sequence, first.sequence.checked_next().unwrap());
        assert_eq!(retry, capture);
    }

    #[test]
    fn exhausted_diagnostic_text_budget_keeps_source_traversal_available() {
        let budget = source_extraction_budget(
            MAX_LENS_SOURCE_NODES,
            0,
            MAX_AX_RESOURCE_REFERENCES,
            MAX_AX_TOTAL_RESOURCE_URI_BYTES,
        )
        .expect("remaining nodes keep extraction available");

        assert_eq!(budget.max_nodes, MAX_LENS_SOURCE_NODES);
        assert_eq!(budget.max_text_bytes, 0);
        assert_eq!(budget.limits.max_text_bytes, 0);
        assert!(source_extraction_budget(
            0,
            MAX_LENS_SOURCE_TEXT_BYTES,
            MAX_AX_RESOURCE_REFERENCES,
            MAX_AX_TOTAL_RESOURCE_URI_BYTES,
        )
        .is_none());
    }
}
