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
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    num::NonZeroU64,
    pin::Pin,
    sync::Arc,
};
use uuid::Uuid;

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
    Legacy,
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

/// Truncates an extraction to the budget it was issued and reports whether anything was
/// dropped along with the text bytes actually retained. The platform adapter copies node
/// attributes verbatim and no adapter limit covers them, so the published context is
/// bounded here.
fn enforce_source_extraction_budget(
    extraction: &mut domain::model::ExtractionResult,
    max_nodes: usize,
    max_text_bytes: usize,
) -> (bool, usize) {
    let mut truncated = truncate_nodes_to_budget(extraction, max_nodes);
    let mut remaining = max_text_bytes;
    // The structured attributes are what the Agent projection is built from, and nothing
    // else bounds them. The flat `text` is a diagnostic duplicate the adapter has already
    // capped at this same limit, so it is charged last: charging it first spent the whole
    // allowance and emptied every title, value and description behind it.
    for node in &mut extraction.nodes {
        for field in [
            node.title.as_mut(),
            node.value.as_mut(),
            node.description.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            truncated |= charge_text(field, &mut remaining);
        }
    }
    truncated |= charge_text(&mut extraction.text, &mut remaining);
    (truncated, max_text_bytes - remaining)
}

/// Keeps at most `max_nodes` nodes and leaves the remaining graph internally consistent.
///
/// Dropping nodes cuts edges: a retained parent keeps `children` entries that no longer
/// exist, and a retained node can lose its parent. `LensDocument::from_accessibility`
/// rejects either as `MissingChild`/`MissingParent`, which would discard the whole
/// structured document — and with it the AX-image plan — rather than keep the safe prefix
/// this bound exists to produce.
fn truncate_nodes_to_budget(
    extraction: &mut domain::model::ExtractionResult,
    max_nodes: usize,
) -> bool {
    if extraction.nodes.len() <= max_nodes {
        return false;
    }
    extraction.nodes.truncate(max_nodes);
    // A node whose parent was dropped cannot be re-rooted: depth is validated against the
    // parent's, so the only consistent repair is to drop the orphaned subtree as well.
    loop {
        let retained = extraction
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        let before = extraction.nodes.len();
        extraction.nodes.retain(|node| {
            node.parent_id
                .as_deref()
                .is_none_or(|parent| retained.contains(parent))
        });
        if extraction.nodes.len() == before {
            break;
        }
    }
    let retained = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    for node in &mut extraction.nodes {
        node.children.retain(|child| retained.contains(child));
    }
    true
}

/// Charges `remaining` for `text`, truncating on a char boundary once it is exhausted.
fn charge_text(text: &mut String, remaining: &mut usize) -> bool {
    if text.len() <= *remaining {
        *remaining -= text.len();
        return false;
    }
    let mut keep = *remaining;
    while keep > 0 && !text.is_char_boundary(keep) {
        keep -= 1;
    }
    text.truncate(keep);
    *remaining = 0;
    true
}

pub struct BuiltContext {
    pub target_set: LensTargetSet,
    pub context: LensContext,
    pub payloads: Vec<LensMediaPayload>,
}

pub async fn build_context(
    executor: &impl BlockingExecutor,
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
        let extraction_budget = source_extraction_budget(
            remaining_nodes,
            remaining_text_bytes,
            remaining_resource_refs,
            remaining_resource_uri_bytes,
        );
        let accessibility = if let Some(extraction_budget) = extraction_budget {
            let max_nodes = extraction_budget.max_nodes;
            let max_text_bytes = extraction_budget.max_text_bytes;
            let limits = extraction_budget.limits;
            let extraction_target = match access_mode {
                WindowAccessMode::Registered => ExtractionTarget::Registered {
                    operation_id,
                    identity: conversion::window_identity(&target.identity),
                },
                WindowAccessMode::Legacy => {
                    ExtractionTarget::Legacy(conversion::selected_window(&target.selected_window()))
                }
            };
            let service = accessibility_service.clone();
            let mut extraction = match executor
                .run(move || {
                    service
                        .extract(extraction_target, limits)
                        .map(conversion::extraction)
                })
                .await
            {
                Ok(Ok(extraction)) => extraction,
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
            // The response is data, not a promise. Bound what is actually retained and
            // charge the group budget with the measured footprint as well as the reported
            // traversal cost, so a metric that under-reports cannot leave the budget full.
            let (truncated, charged_text_bytes) =
                enforce_source_extraction_budget(&mut extraction, max_nodes, max_text_bytes);
            if truncated {
                extraction.diagnostics.push(format!(
                    "{} exceeded the version 1 source extraction budget and was truncated",
                    target.id
                ));
            }
            remaining_nodes = remaining_nodes
                .saturating_sub(extraction.nodes.len().max(extraction.metrics.visited_nodes));
            remaining_text_bytes = remaining_text_bytes
                .saturating_sub(charged_text_bytes.max(extraction.metrics.text_bytes));
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
                context_revision,
                target: current_target.clone(),
                plan,
                remaining_total_bytes: remaining_media_bytes,
                access_mode,
            },
        )
        .await;
        if let Some(observed_frame) = media.observed_window_frame {
            current_target.facts.frame = observed_frame;
            observed_facts.insert(target.id.clone(), current_target.facts.clone());
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
    operation_id: Uuid,
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
        operation_id,
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
                        operation_id,
                        window_id: target.identity.window_id,
                    },
                    NonZeroU64::new(context_revision).ok_or_else(|| {
                        PlatformError::Operation(
                            "registered image capture context revision must be non-zero".into(),
                        )
                    })?,
                ),
                // Preserve the explicit legacy validation path and its revision-one capture namespace.
                WindowAccessMode::Legacy => (
                    CaptureTarget::Legacy {
                        window_id: target.identity.window_id,
                    },
                    NonZeroU64::MIN,
                ),
            };
            capture_media(
                capture_service.as_ref(),
                MediaCaptureContext {
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
    use domain::model::ExtractedNode as DomainNode;

    fn node(id: &str, parent: Option<&str>, depth: usize, order: usize) -> DomainNode {
        DomainNode {
            id: id.into(),
            parent_id: parent.map(Into::into),
            order,
            depth,
            role: None,
            subrole: None,
            title: None,
            value: None,
            description: None,
            bounds: None,
            resource_refs: Vec::new(),
            children: Vec::new(),
        }
    }

    fn extraction_with(nodes: Vec<DomainNode>, text: &str) -> domain::model::ExtractionResult {
        domain::model::ExtractionResult {
            quality: domain::model::ExtractionQuality::Full,
            resolved_window: None,
            nodes,
            text: text.into(),
            metrics: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn structured_attributes_outlive_the_duplicate_flat_text() {
        // The Agent projection is built from the node attributes, not the flat diagnostic
        // text, so spending the allowance on the text would leave the source effectively
        // empty for the only consumer that matters.
        let mut leaf = node("leaf", None, 0, 0);
        leaf.value = Some("kept".into());
        let mut extraction = extraction_with(vec![leaf], "0123456789");
        let (truncated, charged) = enforce_source_extraction_budget(&mut extraction, 16, 6);

        assert!(truncated);
        assert_eq!(charged, 6);
        assert_eq!(extraction.nodes[0].value.as_deref(), Some("kept"));
        assert_eq!(extraction.text, "01");
    }

    #[test]
    fn node_truncation_leaves_a_graph_the_document_still_accepts() {
        // Truncation cuts edges. A retained parent must not keep a dropped child, and a
        // node that lost its parent cannot be re-rooted because depth is validated
        // against it, so the orphaned subtree goes too.
        let mut root = node("root", None, 0, 0);
        root.children = vec!["kept".into(), "dropped".into()];
        let mut kept = node("kept", Some("root"), 1, 1);
        kept.children = vec!["grandchild".into()];
        let nodes = vec![
            root,
            kept,
            node("dropped", Some("root"), 1, 2),
            node("grandchild", Some("kept"), 2, 3),
        ];
        let mut extraction = extraction_with(nodes, "");
        let (truncated, _) = enforce_source_extraction_budget(&mut extraction, 2, 1024);

        assert!(truncated);
        let ids: Vec<&str> = extraction.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["root", "kept"]);
        assert_eq!(extraction.nodes[0].children, ["kept"]);
        assert!(extraction.nodes[1].children.is_empty());
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
        calls: Mutex<Vec<Call>>,
        extractions: Mutex<VecDeque<Result<ExtractionResult, String>>>,
        capture_error: Option<String>,
    }

    impl Accessibility for Sources {
        fn extract(
            &self,
            target: ExtractionTarget,
            limits: ExtractionLimits,
        ) -> Result<ExtractionResult, PlatformError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Accessibility(target, limits));
            self.extractions
                .lock()
                .unwrap()
                .pop_front()
                .expect("declared source result")
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
            let bounds = facts::Bounds {
                x: 12.0,
                y: 24.0,
                width: 200.0,
                height: 100.0,
            };
            Ok(CaptureBatch {
                window_bounds: Some(bounds),
                captures: requests
                    .iter()
                    .map(|request| CapturedImage {
                        attachment_id: request.id.clone(),
                        source_bounds: request.bounds.unwrap_or(bounds),
                        captured_bounds: request.bounds.unwrap_or(bounds),
                        coverage: CaptureCoverage::FullRegion,
                        pixel_width: 1,
                        pixel_height: 1,
                        // Opaque decoded payload: PNG validation belongs to the adapter, not this test service.
                        png: vec![137, 80, 78, 71],
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
                    "window_id":id, "bundle_id":"com.example.context", "pid":42,
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
            "id":"text", "parent_id":null, "order":0, "depth":0, "role":"AXStaticText",
            "value":"Current text", "bounds":null, "children":[]
        })];
        if image {
            nodes.push(json!({
                "id":"image", "parent_id":null, "order":1, "depth":0, "role":"AXImage",
                "description":"Current image",
                "bounds":{"x":30.0,"y":40.0,"width":20.0,"height":10.0}, "children":[]
            }));
        }
        serde_json::from_value(json!({
            "quality":"full",
            "resolved_window":{
                "facts":{"title":"Observed title","application_name":"Fixture",
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
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 3),
        ))
        .unwrap();
        assert!(built.target_set.has_same_identity(&targets));
        for target in &built.target_set.targets {
            assert_eq!(target.facts.title, "Observed title");
            assert_eq!(
                target.facts.frame.x, 12.0,
                "capture frame supersedes the AX frame"
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
            .ends_with("/3/media/media-window-1-image"));
        assert!(built.payloads[1]
            .uri
            .ends_with("/3/media/media-window-2-image"));
        let input =
            LensInput::from_context(&built.context).expect("real graph/media produces an input");
        assert_eq!(input.sources.len(), 2);
        let calls = sources.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        for (index, pair) in calls.as_chunks::<2>().0.iter().enumerate() {
            let id = u32::try_from(index + 1).unwrap();
            let Call::Accessibility(
                ExtractionTarget::Registered {
                    operation_id,
                    identity,
                },
                limits,
            ) = &pair[0]
            else {
                panic!("registered AX precedes capture")
            };
            assert_eq!(*operation_id, targets.selection_id);
            assert_eq!(identity.window_id, id);
            assert_eq!(limits.max_nodes, 30_000);
            assert_eq!(limits.max_resource_refs, 256 - index as u32 * 2);
            assert_eq!(
                limits.max_total_resource_uri_bytes,
                131_072 - index as u32 * 20
            );
            let Call::Capture(target, requests, limits) = &pair[1] else {
                panic!("capture follows AX")
            };
            assert_eq!(
                *target,
                CaptureTarget::Registered {
                    operation_id: targets.selection_id,
                    window_id: id
                }
            );
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].scope, CaptureScope::AxElementRegion);
            assert_eq!(limits.max_total_bytes, 16_777_216 - index as u32 * 4);
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
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Legacy, 7),
        ))
        .unwrap();
        let calls = sources.calls.lock().unwrap();
        assert!(
            matches!(&calls[0], Call::Accessibility(ExtractionTarget::Legacy(window), _) if window == &conversion::selected_window(&targets.targets[0].selected_window()))
        );
        assert!(matches!(
            &calls[1],
            Call::Capture(CaptureTarget::Legacy { window_id: 1 }, _, _)
        ));
        assert!(built.payloads[0]
            .uri
            .ends_with("/7/media/media-window-1-image"));
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
            matches!(&calls[2], Call::Capture(CaptureTarget::Registered {window_id:3, ..}, requests, _) if requests[0].scope == CaptureScope::WindowFallback)
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
            sources.clone(),
            sources.clone(),
            request(&targets, WindowAccessMode::Registered, 1),
        ))
        .unwrap();
        let calls = sources.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        let Call::Accessibility(ExtractionTarget::Registered { identity, .. }, limits) = &calls[2]
        else {
            panic!("third source AX remains scheduled");
        };
        assert_eq!(identity.window_id, 3);
        assert_eq!(limits.max_text_bytes, 0);
        assert_eq!(limits.max_nodes, MAX_LENS_SOURCE_NODES as u32);
        assert!(
            matches!(&calls[3], Call::Capture(CaptureTarget::Registered {window_id:3, ..}, requests, _) if requests[0].scope == CaptureScope::AxElementRegion)
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
