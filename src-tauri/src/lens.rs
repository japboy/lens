use crate::model::{
    Bounds, ExtractedNode, ExtractionMetrics, ExtractionQuality, ExtractionResult,
    ResourceReference, SelectedWindow,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

pub const LENS_DOCUMENT_SCHEMA_VERSION: u32 = 2;
pub const LENS_CONTEXT_SCHEMA_VERSION: u32 = 3;
pub const LENS_INPUT_SCHEMA_VERSION: u32 = 3;
pub const MAX_LENS_INPUT_BYTES: usize = 512 * 1024;
pub const MAX_AX_RESOURCE_REFERENCES: usize = 256;
pub const MAX_AX_RESOURCE_URI_BYTES: usize = 16 * 1024;
pub const MAX_AX_TOTAL_RESOURCE_URI_BYTES: usize = 128 * 1024;
pub const MAX_LENS_MEDIA_ATTACHMENTS: usize = 16;
pub const MAX_LENS_MEDIA_LONG_EDGE: usize = 1_568;
pub const MAX_LENS_MEDIA_PIXELS: usize = 1_150_000;
pub const MAX_LENS_MEDIA_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_LENS_MEDIA_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_LENS_INPUT_FIELD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensSource {
    pub application: String,
    pub window_title: String,
    pub bundle_id: String,
    pub window_id: u32,
}

impl From<&SelectedWindow> for LensSource {
    fn from(target: &SelectedWindow) -> Self {
        Self {
            application: target.application_name.clone(),
            window_title: target.title.clone(),
            bundle_id: target.bundle_id.clone(),
            window_id: target.window_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensNodeKind {
    Heading,
    Paragraph,
    List,
    ListItem,
    Table,
    Row,
    Cell,
    Link,
    Control,
    Dialog,
    Region,
    Text,
    Image,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensCoordinateSpace {
    ScreenPoints,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub order: usize,
    pub depth: usize,
    pub kind: LensNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_space: Option<LensCoordinateSpace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_refs: Vec<ResourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensDocument {
    pub schema_version: u32,
    pub source: LensSource,
    pub roots: Vec<String>,
    pub nodes: BTreeMap<String, LensNode>,
    pub quality: ExtractionQuality,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_resource_ref_count: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LensDocumentError {
    #[error("usable extraction contains no nodes")]
    EmptyUsableDocument,
    #[error("extracted node {0} has no identity")]
    MissingIdentity(usize),
    #[error("extracted node identity is duplicated: {0}")]
    DuplicateIdentity(String),
    #[error("extracted node order is duplicated: {0}")]
    DuplicateOrder(usize),
    #[error("extracted node {node_id} references missing parent {parent_id}")]
    MissingParent { node_id: String, parent_id: String },
    #[error("extracted node {node_id} references missing child {child_id}")]
    MissingChild { node_id: String, child_id: String },
    #[error("extracted edge is inconsistent: parent {parent_id}, child {child_id}")]
    InconsistentEdge { parent_id: String, child_id: String },
    #[error("extracted node {node_id} has depth {actual}; expected {expected}")]
    InconsistentDepth {
        node_id: String,
        actual: usize,
        expected: usize,
    },
}

impl LensDocument {
    pub fn from_accessibility(
        target: &SelectedWindow,
        extraction: &ExtractionResult,
    ) -> Result<Option<Self>, LensDocumentError> {
        if extraction.quality == ExtractionQuality::Unavailable {
            return Ok(None);
        }
        if extraction.nodes.is_empty() {
            return Err(LensDocumentError::EmptyUsableDocument);
        }

        let mut nodes = BTreeMap::new();
        let mut orders = BTreeSet::new();
        for (index, extracted) in extraction.nodes.iter().enumerate() {
            if extracted.id.is_empty() {
                return Err(LensDocumentError::MissingIdentity(index));
            }
            if !orders.insert(extracted.order) {
                return Err(LensDocumentError::DuplicateOrder(extracted.order));
            }
            let node = LensNode::from(extracted);
            if nodes.insert(node.id.clone(), node).is_some() {
                return Err(LensDocumentError::DuplicateIdentity(extracted.id.clone()));
            }
        }

        let mut roots = Vec::new();
        for node in nodes.values() {
            match node.parent_id.as_ref() {
                Some(parent_id) => {
                    let parent =
                        nodes
                            .get(parent_id)
                            .ok_or_else(|| LensDocumentError::MissingParent {
                                node_id: node.id.clone(),
                                parent_id: parent_id.clone(),
                            })?;
                    if !parent.children.contains(&node.id) {
                        return Err(LensDocumentError::InconsistentEdge {
                            parent_id: parent_id.clone(),
                            child_id: node.id.clone(),
                        });
                    }
                    let expected_depth = parent.depth.saturating_add(1);
                    if node.depth != expected_depth {
                        return Err(LensDocumentError::InconsistentDepth {
                            node_id: node.id.clone(),
                            actual: node.depth,
                            expected: expected_depth,
                        });
                    }
                }
                None => roots.push(node.id.clone()),
            }
            for child_id in &node.children {
                let child = nodes
                    .get(child_id)
                    .ok_or_else(|| LensDocumentError::MissingChild {
                        node_id: node.id.clone(),
                        child_id: child_id.clone(),
                    })?;
                if child.parent_id.as_deref() != Some(node.id.as_str()) {
                    return Err(LensDocumentError::InconsistentEdge {
                        parent_id: node.id.clone(),
                        child_id: child_id.clone(),
                    });
                }
            }
        }
        roots.sort_by_key(|id| nodes.get(id).map_or(usize::MAX, |node| node.order));

        Ok(Some(Self {
            schema_version: LENS_DOCUMENT_SCHEMA_VERSION,
            source: LensSource::from(target),
            roots,
            nodes,
            quality: extraction.quality,
            omitted_resource_ref_count: extraction.metrics.omitted_resource_refs,
            diagnostics: extraction.diagnostics.clone(),
        }))
    }

    fn attach_media(&mut self, attachments: &[LensMediaAttachment]) {
        for attachment in attachments {
            let Some(node_id) = attachment.source_node_id.as_ref() else {
                continue;
            };
            if let Some(node) = self.nodes.get_mut(node_id) {
                node.media_refs.push(attachment.id.clone());
            }
        }
    }

    fn required_projection_nodes(&self, attachments: &[LensMediaAttachment]) -> BTreeSet<String> {
        let mut required = BTreeSet::new();
        let resource_node_ids = self
            .nodes
            .values()
            .filter(|node| !node.resource_refs.is_empty())
            .map(|node| node.id.as_str());
        let media_node_ids = attachments
            .iter()
            .filter_map(|attachment| attachment.source_node_id.as_deref());
        for mut node_id in resource_node_ids.chain(media_node_ids) {
            while required.insert(node_id.to_string()) {
                let Some(parent_id) = self
                    .nodes
                    .get(node_id)
                    .and_then(|node| node.parent_id.as_deref())
                else {
                    break;
                };
                node_id = parent_id;
            }
        }
        required
    }

    fn project(
        &self,
        required: &BTreeSet<String>,
        optional_limit: usize,
    ) -> (LensDocumentProjection, Vec<ProjectionOmission>) {
        let mut ordered = self.nodes.values().collect::<Vec<_>>();
        ordered.sort_by_key(|node| node.order);
        let mut optional_seen = 0;
        let mut included = BTreeSet::new();
        for node in &ordered {
            if required.contains(&node.id) || optional_seen < optional_limit {
                included.insert(node.id.clone());
            }
            if !required.contains(&node.id) {
                optional_seen += 1;
            }
        }

        let mut omitted_root_chrome = false;
        let mut truncated_fields = false;
        let nodes = ordered
            .iter()
            .filter(|node| included.contains(&node.id))
            .map(|node| {
                let is_application_root =
                    node.parent_id.is_none() && node.role.as_deref() == Some("AXWindow");
                omitted_root_chrome |= is_application_root
                    && (node.title.is_some() || node.value.is_some() || node.description.is_some());
                LensContentNode {
                    id: node.id.clone(),
                    parent_id: node.parent_id.clone().filter(|id| included.contains(id)),
                    kind: node.kind,
                    role: matches!(node.kind, LensNodeKind::Unknown | LensNodeKind::Image)
                        .then(|| node.role.clone())
                        .flatten(),
                    subrole: node.subrole.clone(),
                    title: (!is_application_root)
                        .then(|| project_text(node.title.as_deref(), &mut truncated_fields))
                        .flatten(),
                    value: (!is_application_root)
                        .then(|| project_text(node.value.as_deref(), &mut truncated_fields))
                        .flatten(),
                    description: (!is_application_root)
                        .then(|| project_text(node.description.as_deref(), &mut truncated_fields))
                        .flatten(),
                    media_refs: node.media_refs.clone(),
                    resource_refs: node.resource_refs.clone(),
                }
            })
            .collect::<Vec<_>>();

        let mut omissions = Vec::new();
        if omitted_root_chrome {
            omissions.push(ProjectionOmission::document_wide(
                ProjectionOmissionReason::ApplicationChrome,
                "Root AXWindow text is represented by source metadata instead of document content.",
            ));
        }
        if truncated_fields {
            omissions.push(ProjectionOmission::document_wide(
                ProjectionOmissionReason::TokenBudget,
                "One or more individual text fields were truncated at the versioned 16 KiB UTF-8 field limit.",
            ));
        }
        if self.omitted_resource_ref_count > 0 {
            omissions.push(ProjectionOmission {
                reason: ProjectionOmissionReason::ResourceBudget,
                omitted_node_count: 0,
                first_order: None,
                last_order: None,
                detail: Some(format!(
                    "{} URI resource references were omitted whole at the finite extraction boundary; retained URIs were never truncated.",
                    self.omitted_resource_ref_count
                )),
            });
        }
        let omitted = ordered
            .iter()
            .filter(|node| !included.contains(&node.id))
            .collect::<Vec<_>>();
        if let (Some(first), Some(last)) = (omitted.first(), omitted.last()) {
            omissions.push(ProjectionOmission {
                reason: ProjectionOmissionReason::TokenBudget,
                omitted_node_count: omitted.len(),
                first_order: Some(first.order),
                last_order: Some(last.order),
                detail: Some(format!(
                    "Nodes were omitted to keep compact LensInput at or below {MAX_LENS_INPUT_BYTES} UTF-8 bytes."
                )),
            });
        }

        (LensDocumentProjection { nodes }, omissions)
    }
}

impl From<&ExtractedNode> for LensNode {
    fn from(node: &ExtractedNode) -> Self {
        Self {
            id: node.id.clone(),
            parent_id: node.parent_id.clone(),
            order: node.order,
            depth: node.depth,
            kind: normalize_kind(node.role.as_deref()),
            role: node.role.clone(),
            subrole: node.subrole.clone(),
            title: node.title.clone(),
            value: node.value.clone(),
            description: node.description.clone(),
            bounds: node.bounds,
            coordinate_space: node.bounds.map(|_| LensCoordinateSpace::ScreenPoints),
            children: node.children.clone(),
            media_refs: Vec::new(),
            resource_refs: node.resource_refs.clone(),
        }
    }
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn normalize_kind(role: Option<&str>) -> LensNodeKind {
    match role.unwrap_or_default().to_ascii_lowercase().as_str() {
        "axheading" => LensNodeKind::Heading,
        "axstatictext" | "axtext" => LensNodeKind::Text,
        "axlist" => LensNodeKind::List,
        "axlistitem" => LensNodeKind::ListItem,
        "axtable" | "axoutline" => LensNodeKind::Table,
        "axrow" => LensNodeKind::Row,
        "axcell" | "axcolumn" => LensNodeKind::Cell,
        "axlink" => LensNodeKind::Link,
        "axbutton" | "axcheckbox" | "axradiobutton" | "axtextfield" | "axtextarea"
        | "axcombobox" | "axpopupbutton" | "axslider" | "axswitch" => LensNodeKind::Control,
        "axdialog" | "axsheet" => LensNodeKind::Dialog,
        "axgroup" | "axsection" | "axlandmark" | "axwebarea" | "axwindow" => LensNodeKind::Region,
        "axparagraph" => LensNodeKind::Paragraph,
        "aximage" => LensNodeKind::Image,
        _ => LensNodeKind::Unknown,
    }
}

fn project_text(value: Option<&str>, truncated: &mut bool) -> Option<String> {
    let value = value?;
    if value.len() <= MAX_LENS_INPUT_FIELD_BYTES {
        return Some(value.to_string());
    }
    let mut boundary = MAX_LENS_INPUT_FIELD_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    *truncated = true;
    Some(value[..boundary].to_string())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensMediaScope {
    AxElementRegion,
    WindowFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensMediaCoverage {
    FullRegion,
    VisibleSubregion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensMediaAttachment {
    pub id: String,
    pub uri: String,
    pub scope: LensMediaScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
    pub source_bounds: Bounds,
    pub captured_bounds: Bounds,
    pub coverage: LensMediaCoverage,
    pub coordinate_space: LensCoordinateSpace,
    pub mime_type: String,
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub encoded_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensMediaPayload {
    pub attachment_id: String,
    pub uri: String,
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LensMediaOmissionReason {
    MissingBounds,
    InvalidBounds,
    OutsideWindow,
    AttachmentLimit,
    ByteBudget,
    CaptureFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensMediaOmission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
    pub reason: LensMediaOmissionReason,
    pub omitted_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_order: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_order: Option<usize>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensMediaRequest {
    pub id: String,
    pub scope: LensMediaScope,
    pub source_node_id: Option<String>,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensMediaPlan {
    pub requests: Vec<LensMediaRequest>,
    pub omissions: Vec<LensMediaOmission>,
}

impl LensMediaPlan {
    pub fn from_accessibility(target: &SelectedWindow, extraction: &ExtractionResult) -> Self {
        if extraction.quality == ExtractionQuality::Unavailable
            || !matches!(
                LensDocument::from_accessibility(target, extraction),
                Ok(Some(_))
            )
        {
            return Self {
                requests: vec![LensMediaRequest {
                    id: "media-window-fallback".into(),
                    scope: LensMediaScope::WindowFallback,
                    source_node_id: None,
                    bounds: None,
                }],
                omissions: Vec::new(),
            };
        }

        let mut image_nodes = extraction
            .nodes
            .iter()
            .filter(|node| {
                node.role
                    .as_deref()
                    .is_some_and(|role| role.eq_ignore_ascii_case("AXImage"))
            })
            .collect::<Vec<_>>();
        image_nodes.sort_by_key(|node| node.order);
        let mut plan = Self::default();
        let mut planning_omissions = BTreeMap::new();
        for node in image_nodes {
            let attachment_id = format!("media-{}", node.id);
            let Some(bounds) = node.bounds else {
                record_planning_omission(
                    &mut planning_omissions,
                    LensMediaOmissionReason::MissingBounds,
                    node.order,
                );
                continue;
            };
            if !valid_bounds(bounds) {
                record_planning_omission(
                    &mut planning_omissions,
                    LensMediaOmissionReason::InvalidBounds,
                    node.order,
                );
                continue;
            }
            if !bounds_intersect(bounds, target.frame) {
                record_planning_omission(
                    &mut planning_omissions,
                    LensMediaOmissionReason::OutsideWindow,
                    node.order,
                );
                continue;
            }
            if plan.requests.len() >= MAX_LENS_MEDIA_ATTACHMENTS {
                record_planning_omission(
                    &mut planning_omissions,
                    LensMediaOmissionReason::AttachmentLimit,
                    node.order,
                );
                continue;
            }
            plan.requests.push(LensMediaRequest {
                id: attachment_id,
                scope: LensMediaScope::AxElementRegion,
                source_node_id: Some(node.id.clone()),
                bounds: Some(bounds),
            });
        }
        plan.omissions = planning_omissions
            .into_iter()
            .map(
                |(reason, (omitted_count, first_order, last_order))| LensMediaOmission {
                    attachment_id: None,
                    source_node_id: None,
                    reason,
                    omitted_count,
                    first_order: Some(first_order),
                    last_order: Some(last_order),
                    detail: planning_omission_detail(reason, omitted_count),
                },
            )
            .collect();
        plan
    }
}

fn record_planning_omission(
    omissions: &mut BTreeMap<LensMediaOmissionReason, (usize, usize, usize)>,
    reason: LensMediaOmissionReason,
    order: usize,
) {
    let range = omissions.entry(reason).or_insert((0, order, order));
    range.0 = range.0.saturating_add(1);
    range.2 = order;
}

fn planning_omission_detail(reason: LensMediaOmissionReason, count: usize) -> String {
    match reason {
        LensMediaOmissionReason::MissingBounds => format!(
            "{count} AXImage nodes expose no screen bounds, so their pixels cannot be captured deterministically."
        ),
        LensMediaOmissionReason::InvalidBounds => format!(
            "{count} AXImage nodes expose non-finite or non-positive screen bounds."
        ),
        LensMediaOmissionReason::OutsideWindow => format!(
            "{count} AXImage nodes do not intersect the selected-window frame, so they have no visible pixels to capture."
        ),
        LensMediaOmissionReason::AttachmentLimit => format!(
            "{count} AXImage regions were omitted after the versioned {MAX_LENS_MEDIA_ATTACHMENTS}-attachment limit."
        ),
        LensMediaOmissionReason::ByteBudget | LensMediaOmissionReason::CaptureFailed => {
            unreachable!("native capture outcomes are not planning omissions")
        }
    }
}

fn bounds_intersect(left: Bounds, right: Bounds) -> bool {
    valid_bounds(left)
        && valid_bounds(right)
        && left.x < right.x + right.width
        && right.x < left.x + left.width
        && left.y < right.y + right.height
        && right.y < left.y + left.height
}

fn valid_bounds(bounds: Bounds) -> bool {
    [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f64::is_finite)
        && bounds.width > 0.0
        && bounds.height > 0.0
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensMediaCapture {
    pub attachments: Vec<LensMediaAttachment>,
    pub payloads: Vec<LensMediaPayload>,
    pub omissions: Vec<LensMediaOmission>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensAccessibilitySource {
    pub source_id: String,
    pub target_id: String,
    pub revision: u64,
    pub source: LensSource,
    pub capture: ExtractionResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<LensDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensContext {
    pub schema_version: u32,
    pub context_id: Uuid,
    pub revision: u64,
    pub accessibility: LensAccessibilitySource,
    pub media: Vec<LensMediaAttachment>,
    pub media_omissions: Vec<LensMediaOmission>,
    pub quality: ExtractionQuality,
    pub diagnostics: Vec<String>,
}

impl LensContext {
    pub fn from_capture(
        context_id: Uuid,
        target: &SelectedWindow,
        mut accessibility: ExtractionResult,
        mut media: LensMediaCapture,
    ) -> Self {
        let mut document = match LensDocument::from_accessibility(target, &accessibility) {
            Ok(document) => document,
            Err(error) => {
                accessibility.diagnostics.push(format!(
                    "unable to normalize Accessibility structure: {error}"
                ));
                None
            }
        };
        if let Some(document) = document.as_mut() {
            document.attach_media(&media.attachments);
        }

        let has_document = document.is_some();
        let has_window_fallback = media
            .attachments
            .iter()
            .any(|attachment| attachment.scope == LensMediaScope::WindowFallback);
        let quality = if has_document {
            let has_partial_media = media
                .attachments
                .iter()
                .any(|attachment| attachment.coverage == LensMediaCoverage::VisibleSubregion);
            if accessibility.quality == ExtractionQuality::Full
                && media.omissions.is_empty()
                && !has_partial_media
            {
                ExtractionQuality::Full
            } else {
                ExtractionQuality::Partial
            }
        } else if has_window_fallback {
            ExtractionQuality::Partial
        } else {
            ExtractionQuality::Unavailable
        };

        let target_id = target_id(target);
        let source_id = format!("{target_id}:accessibility");
        let mut diagnostics = accessibility
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{source_id}: {diagnostic}"))
            .collect::<Vec<_>>();
        diagnostics.append(&mut media.diagnostics);
        diagnostics.extend(media.omissions.iter().map(|omission| {
            format!(
                "media {}: {}",
                omission.attachment_id.as_deref().unwrap_or("unidentified"),
                omission.detail
            )
        }));
        diagnostics.extend(
            media
                .attachments
                .iter()
                .filter(|attachment| {
                    attachment.coverage == LensMediaCoverage::VisibleSubregion
                })
                .map(|attachment| {
                    format!(
                        "media {}: only the selected-window-visible subregion of the AX image could be captured",
                        attachment.id
                    )
                }),
        );

        Self {
            schema_version: LENS_CONTEXT_SCHEMA_VERSION,
            context_id,
            revision: 1,
            accessibility: LensAccessibilitySource {
                source_id,
                target_id,
                revision: 1,
                source: LensSource::from(target),
                capture: accessibility,
                document,
            },
            media: media.attachments,
            media_omissions: media.omissions,
            quality,
            diagnostics,
        }
    }

    pub fn unavailable_accessibility(message: impl Into<String>) -> ExtractionResult {
        ExtractionResult {
            quality: ExtractionQuality::Unavailable,
            resolved_window: None,
            nodes: Vec::new(),
            text: String::new(),
            metrics: ExtractionMetrics::default(),
            diagnostics: vec![message.into()],
        }
    }

    pub fn accessibility_capture(&self) -> &ExtractionResult {
        &self.accessibility.capture
    }
}

fn target_id(target: &SelectedWindow) -> String {
    format!("macos:{}:{}", target.bundle_id, target.window_id)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensDocumentProjection {
    pub nodes: Vec<LensContentNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensContentNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub kind: LensNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_refs: Vec<ResourceReference>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOmissionReason {
    ApplicationChrome,
    TokenBudget,
    UnsupportedSemantics,
    ResourceBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionOmission {
    pub reason: ProjectionOmissionReason,
    pub omitted_node_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_order: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_order: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ProjectionOmission {
    fn document_wide(reason: ProjectionOmissionReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            omitted_node_count: 0,
            first_order: None,
            last_order: None,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensInputSource {
    pub source_id: String,
    pub target_id: String,
    pub source_revision: u64,
    pub source: LensSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<LensDocumentProjection>,
    pub quality: ExtractionQuality,
    pub omissions: Vec<ProjectionOmission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensInput {
    pub schema_version: u32,
    pub context_id: Uuid,
    pub context_revision: u64,
    pub sources: Vec<LensInputSource>,
    pub media: Vec<LensMediaAttachment>,
    pub media_omissions: Vec<LensMediaOmission>,
    pub quality: ExtractionQuality,
}

impl LensInput {
    pub fn from_context(context: &LensContext) -> Option<Self> {
        let document = context.accessibility.document.as_ref();
        if document.is_none() && context.media.is_empty() {
            return None;
        }
        let required = document
            .map(|document| document.required_projection_nodes(&context.media))
            .unwrap_or_default();
        let optional_count = document.map_or(0, |document| {
            document
                .nodes
                .keys()
                .filter(|id| !required.contains(*id))
                .count()
        });
        let build = |optional_limit: usize| {
            let (projection, omissions) = document.map_or_else(
                || {
                    (
                        None,
                        vec![ProjectionOmission::document_wide(
                            ProjectionOmissionReason::UnsupportedSemantics,
                            "Accessibility extraction was unavailable; the attached whole-window bitmap is the explicit fallback.",
                        )],
                    )
                },
                |document| {
                    let (projection, omissions) = document.project(&required, optional_limit);
                    (Some(projection), omissions)
                },
            );
            Self {
                schema_version: LENS_INPUT_SCHEMA_VERSION,
                context_id: context.context_id,
                context_revision: context.revision,
                sources: vec![LensInputSource {
                    source_id: context.accessibility.source_id.clone(),
                    target_id: context.accessibility.target_id.clone(),
                    source_revision: context.accessibility.revision,
                    source: context.accessibility.source.clone(),
                    document: projection,
                    quality: context.accessibility.capture.quality,
                    omissions,
                }],
                media: context.media.clone(),
                media_omissions: context.media_omissions.clone(),
                quality: context.quality,
            }
        };

        let full = build(optional_count);
        if full.serialized_len().ok()? <= MAX_LENS_INPUT_BYTES {
            return Some(full);
        }

        let mut low = 0;
        let mut high = optional_count;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if build(middle).serialized_len().ok()? <= MAX_LENS_INPUT_BYTES {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let bounded = build(low);
        (bounded.serialized_len().ok()? <= MAX_LENS_INPUT_BYTES).then_some(bounded)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn to_markdown(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "## PersonalLens context\n\n```json\n{}\n```",
            self.to_json()?
        ))
    }

    pub fn serialized_len(&self) -> Result<usize, serde_json::Error> {
        serde_json::to_vec(self).map(|json| json.len())
    }

    pub fn contains_text(&self, expected: &str) -> bool {
        self.sources.iter().any(|source| {
            source.document.as_ref().is_some_and(|document| {
                document.nodes.iter().any(|node| {
                    [&node.title, &node.value, &node.description]
                        .into_iter()
                        .flatten()
                        .any(|text| text.contains(expected))
                })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> SelectedWindow {
        SelectedWindow {
            window_id: 7,
            title: "Document".into(),
            application_name: "Browser".into(),
            bundle_id: "example.browser".into(),
            pid: 42,
            frame: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        }
    }

    fn accessibility(quality: ExtractionQuality) -> ExtractionResult {
        ExtractionResult {
            quality,
            resolved_window: None,
            nodes: vec![
                ExtractedNode {
                    id: "node-000000".into(),
                    parent_id: None,
                    order: 0,
                    depth: 0,
                    role: Some("AXWindow".into()),
                    subrole: None,
                    title: Some("Document".into()),
                    value: None,
                    description: None,
                    bounds: Some(target().frame),
                    resource_refs: vec![],
                    children: vec!["node-000001".into(), "node-000002".into()],
                },
                ExtractedNode {
                    id: "node-000001".into(),
                    parent_id: Some("node-000000".into()),
                    order: 1,
                    depth: 1,
                    role: Some("AXStaticText".into()),
                    subrole: None,
                    title: None,
                    value: Some("Repeated".into()),
                    description: None,
                    bounds: None,
                    resource_refs: vec![],
                    children: vec![],
                },
                ExtractedNode {
                    id: "node-000002".into(),
                    parent_id: Some("node-000000".into()),
                    order: 2,
                    depth: 1,
                    role: Some("AXImage".into()),
                    subrole: None,
                    title: None,
                    value: None,
                    description: Some("Sales chart".into()),
                    bounds: Some(Bounds {
                        x: 10.0,
                        y: 20.0,
                        width: 60.0,
                        height: 40.0,
                    }),
                    resource_refs: vec![ResourceReference {
                        uri: "https://example.test/chart.png".into(),
                        source_attribute: "AXURL".into(),
                    }],
                    children: vec![],
                },
            ],
            text: String::new(),
            metrics: ExtractionMetrics::default(),
            diagnostics: vec![],
        }
    }

    fn captured_region() -> LensMediaCapture {
        let attachment = LensMediaAttachment {
            id: "media-node-000002".into(),
            uri: "personallens://context/00000000-0000-0000-0000-000000000000/1/media/media-node-000002".into(),
            scope: LensMediaScope::AxElementRegion,
            source_node_id: Some("node-000002".into()),
            source_bounds: Bounds {
                x: 10.0,
                y: 20.0,
                width: 60.0,
                height: 40.0,
            },
            captured_bounds: Bounds {
                x: 10.0,
                y: 20.0,
                width: 60.0,
                height: 40.0,
            },
            coverage: LensMediaCoverage::FullRegion,
            coordinate_space: LensCoordinateSpace::ScreenPoints,
            mime_type: "image/png".into(),
            pixel_width: 120,
            pixel_height: 80,
            encoded_bytes: 67,
        };
        LensMediaCapture {
            attachments: vec![attachment.clone()],
            payloads: vec![LensMediaPayload {
                attachment_id: attachment.id,
                uri: attachment.uri,
                mime_type: attachment.mime_type,
                data: "iVBORw0KGgo=".into(),
            }],
            ..LensMediaCapture::default()
        }
    }

    #[test]
    fn accessibility_normalization_preserves_graph_and_maps_ax_image() {
        let document =
            LensDocument::from_accessibility(&target(), &accessibility(ExtractionQuality::Full))
                .expect("valid graph")
                .expect("usable document");

        assert_eq!(document.roots, vec!["node-000000"]);
        assert_eq!(document.nodes["node-000001"].kind, LensNodeKind::Text);
        assert_eq!(document.nodes["node-000002"].kind, LensNodeKind::Image);
        assert_eq!(
            document.nodes["node-000002"].resource_refs[0].uri,
            "https://example.test/chart.png"
        );
    }

    #[test]
    fn inconsistent_edges_fail_instead_of_becoming_implicit_structure() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[0].children.clear();

        assert_eq!(
            LensDocument::from_accessibility(&target(), &extraction),
            Err(LensDocumentError::InconsistentEdge {
                parent_id: "node-000000".into(),
                child_id: "node-000001".into(),
            })
        );
    }

    #[test]
    fn usable_ax_requests_only_ax_image_regions() {
        let plan =
            LensMediaPlan::from_accessibility(&target(), &accessibility(ExtractionQuality::Full));

        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].scope, LensMediaScope::AxElementRegion);
        assert_eq!(
            plan.requests[0].source_node_id.as_deref(),
            Some("node-000002")
        );
        assert_eq!(plan.requests[0].bounds.expect("bounds").width, 60.0);
    }

    #[test]
    fn unavailable_ax_requests_one_whole_window_fallback() {
        let plan = LensMediaPlan::from_accessibility(
            &target(),
            &LensContext::unavailable_accessibility("AX unavailable"),
        );

        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].scope, LensMediaScope::WindowFallback);
        assert_eq!(plan.requests[0].bounds, None);
    }

    #[test]
    fn off_window_ax_image_is_omitted_before_capture_planning() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[2].bounds = Some(Bounds {
            x: 110.0,
            y: 20.0,
            width: 60.0,
            height: 40.0,
        });

        let plan = LensMediaPlan::from_accessibility(&target(), &extraction);

        assert!(plan.requests.is_empty());
        assert_eq!(plan.omissions.len(), 1);
        assert_eq!(plan.omissions[0].omitted_count, 1);
        assert_eq!(
            plan.omissions[0].reason,
            LensMediaOmissionReason::OutsideWindow
        );
    }

    #[test]
    fn attachment_limit_is_one_bounded_self_describing_omission_group() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[0].children.clear();
        extraction.nodes.truncate(1);
        for index in 1..101 {
            let id = format!("node-{index:06}");
            extraction.nodes[0].children.push(id.clone());
            extraction.nodes.push(ExtractedNode {
                id,
                parent_id: Some("node-000000".into()),
                order: index,
                depth: 1,
                role: Some("AXImage".into()),
                subrole: None,
                title: None,
                value: None,
                description: None,
                bounds: Some(Bounds {
                    x: 10.0,
                    y: 20.0,
                    width: 60.0,
                    height: 40.0,
                }),
                resource_refs: vec![],
                children: vec![],
            });
        }

        let plan = LensMediaPlan::from_accessibility(&target(), &extraction);

        assert_eq!(plan.requests.len(), MAX_LENS_MEDIA_ATTACHMENTS);
        assert_eq!(plan.omissions.len(), 1);
        assert_eq!(
            plan.omissions[0].reason,
            LensMediaOmissionReason::AttachmentLimit
        );
        assert_eq!(plan.omissions[0].omitted_count, 84);
        assert_eq!(plan.omissions[0].first_order, Some(17));
        assert_eq!(plan.omissions[0].last_order, Some(100));
    }

    #[test]
    fn invalid_ax_graph_uses_the_whole_window_fallback() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[0].children.clear();

        let plan = LensMediaPlan::from_accessibility(&target(), &extraction);

        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].scope, LensMediaScope::WindowFallback);
    }

    #[test]
    fn region_attachment_is_linked_to_its_ax_node_and_input_is_compact() {
        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            accessibility(ExtractionQuality::Full),
            captured_region(),
        );
        let input = LensInput::from_context(&context).expect("usable input");
        let document = input.sources[0].document.as_ref().expect("AX document");
        let image = document
            .nodes
            .iter()
            .find(|node| node.id == "node-000002")
            .expect("image node");

        assert_eq!(context.quality, ExtractionQuality::Full);
        assert_eq!(image.media_refs, vec!["media-node-000002"]);
        assert_eq!(image.resource_refs[0].uri, "https://example.test/chart.png");
        assert_eq!(input.media.len(), 1);
        assert!(input.serialized_len().expect("serialize") <= MAX_LENS_INPUT_BYTES);
        assert!(!input.to_json().expect("compact JSON").contains("\n  "));
    }

    #[test]
    fn visible_subregion_is_explicit_and_makes_the_context_partial() {
        let mut capture = captured_region();
        capture.attachments[0].source_bounds = Bounds {
            x: 0.0,
            y: 10.0,
            width: 80.0,
            height: 60.0,
        };
        capture.attachments[0].coverage = LensMediaCoverage::VisibleSubregion;

        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            accessibility(ExtractionQuality::Full),
            capture,
        );
        let input = LensInput::from_context(&context).expect("partial image input");

        assert_eq!(context.quality, ExtractionQuality::Partial);
        assert_eq!(input.media[0].coverage, LensMediaCoverage::VisibleSubregion);
        assert_ne!(input.media[0].source_bounds, input.media[0].captured_bounds);
        assert!(context
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("visible subregion")));
    }

    #[test]
    fn uri_bearing_node_and_ancestor_survive_compact_projection() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[2].resource_refs.clear();
        extraction.nodes[0].children.clear();
        extraction.nodes.truncate(1);
        for index in 1..8_000 {
            let id = format!("node-{index:06}");
            extraction.nodes[0].children.push(id.clone());
            extraction.nodes.push(ExtractedNode {
                id,
                parent_id: Some("node-000000".into()),
                order: index,
                depth: 1,
                role: Some("AXStaticText".into()),
                subrole: None,
                title: None,
                value: Some(format!("bounded content {index:06}")),
                description: None,
                bounds: None,
                resource_refs: (index == 7_999)
                    .then(|| ResourceReference {
                        uri: "custom-scheme://resource/final".into(),
                        source_attribute: "AXURL".into(),
                    })
                    .into_iter()
                    .collect(),
                children: vec![],
            });
        }
        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            extraction,
            LensMediaCapture::default(),
        );
        let input = LensInput::from_context(&context).expect("bounded URI input");
        let nodes = &input.sources[0].document.as_ref().expect("document").nodes;

        assert!(input.serialized_len().expect("serialize") <= MAX_LENS_INPUT_BYTES);
        assert!(nodes.iter().any(|node| node.id == "node-000000"));
        let resource_node = nodes
            .iter()
            .find(|node| node.id == "node-007999")
            .expect("required URI node");
        assert_eq!(
            resource_node.resource_refs[0].uri,
            "custom-scheme://resource/final"
        );
    }

    #[test]
    fn resource_budget_omission_is_agent_visible() {
        let mut extraction = accessibility(ExtractionQuality::Partial);
        extraction.metrics.omitted_resource_refs = 3;
        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            extraction,
            LensMediaCapture::default(),
        );
        let input = LensInput::from_context(&context).expect("partial URI input");
        let omission = input.sources[0]
            .omissions
            .iter()
            .find(|omission| omission.reason == ProjectionOmissionReason::ResourceBudget)
            .expect("resource omission");

        assert_eq!(omission.omitted_node_count, 0);
        assert!(omission.detail.as_deref().is_some_and(|detail| {
            detail.contains("3 URI") && detail.contains("never truncated")
        }));
    }

    #[test]
    fn whole_window_media_keeps_unavailable_ax_as_explicit_partial_fallback() {
        let mut capture = captured_region();
        capture.attachments[0].scope = LensMediaScope::WindowFallback;
        capture.attachments[0].source_node_id = None;
        capture.payloads[0].attachment_id = capture.attachments[0].id.clone();
        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            LensContext::unavailable_accessibility("AX unavailable"),
            capture,
        );
        let input = LensInput::from_context(&context).expect("image fallback input");

        assert_eq!(context.quality, ExtractionQuality::Partial);
        assert!(input.sources[0].document.is_none());
        assert_eq!(input.media[0].scope, LensMediaScope::WindowFallback);
    }

    #[test]
    fn projection_enforces_the_compact_input_budget() {
        let mut extraction = accessibility(ExtractionQuality::Full);
        extraction.nodes[0].children.clear();
        extraction.nodes.truncate(1);
        for index in 1..8_000 {
            let id = format!("node-{index:06}");
            extraction.nodes[0].children.push(id.clone());
            extraction.nodes.push(ExtractedNode {
                id,
                parent_id: Some("node-000000".into()),
                order: index,
                depth: 1,
                role: Some("AXStaticText".into()),
                subrole: None,
                title: None,
                value: Some(format!("bounded content {index:06}")),
                description: None,
                bounds: None,
                resource_refs: vec![],
                children: vec![],
            });
        }
        let context = LensContext::from_capture(
            Uuid::nil(),
            &target(),
            extraction,
            LensMediaCapture::default(),
        );
        let input = LensInput::from_context(&context).expect("bounded input");

        assert!(input.serialized_len().expect("serialize") <= MAX_LENS_INPUT_BYTES);
        assert!(input.sources[0]
            .omissions
            .iter()
            .any(|omission| omission.reason == ProjectionOmissionReason::TokenBudget));
    }
}
