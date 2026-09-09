//! Canonical semantic projection, independent of observation and Agent delivery state.

use crate::{
    geometry::{Rect, TaggedRect},
    lens::{
        LensContentNode, LensDocumentProjection, LensInput, LensMediaCoverage,
        LensMediaOmissionReason, LensMediaPayload, LensMediaScope, LensTargetSet,
        ProjectionOmission,
    },
    model::{Bounds, ExtractionQuality},
};
use base64::prelude::*;
use serde::{de, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU64,
    str::FromStr,
};
use thiserror::Error;

/// The largest integer that every RFC 8785/I-JSON implementation can represent exactly.
pub const MAX_EXACT_I_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Canonical RFC 8785 bytes and the SHA-256 identity computed over exactly those bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalProjection {
    bytes: Vec<u8>,
    digest: ProjectionDigest,
}

impl CanonicalProjection {
    /// Serializes a semantic projection using RFC 8785 and computes its content identity.
    ///
    /// The additional I-JSON integer check prevents a Rust integer outside the exact IEEE-754
    /// range from being silently rounded by the RFC 8785 number representation.
    pub fn from_serializable<T: Serialize>(value: &T) -> Result<Self, CanonicalProjectionError> {
        // Canonicalize the original value first so non-finite floats are rejected instead of
        // becoming serde_json::Value::Null during an intermediate conversion.
        let bytes = serde_json_canonicalizer::to_vec(value)
            .map_err(CanonicalProjectionError::Canonicalization)?;
        let json = serde_json::to_value(value).map_err(CanonicalProjectionError::Serialization)?;
        validate_i_json_numbers(&json, "")?;
        let value_bytes = serde_json_canonicalizer::to_vec(&json)
            .map_err(CanonicalProjectionError::Canonicalization)?;
        if bytes != value_bytes {
            return Err(CanonicalProjectionError::NonDeterministicSerialization);
        }

        let digest = ProjectionDigest(hex_digest(&Sha256::digest(&bytes)));
        Ok(Self { bytes, digest })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest(&self) -> &ProjectionDigest {
        &self.digest
    }

    pub fn projection_ref(&self, revision: NonZeroU64) -> ProjectionRef {
        ProjectionRef::new(revision, self.digest.clone())
    }
}

/// Deterministic Agent-bound semantic projection and its exact prompt media.
///
/// Transport identities, source revisions, screen origin, and operation-scoped media URIs are
/// deliberately absent from the canonical payload. The canonical bytes are also the JSON sent to
/// the Agent, so the digest and the Agent-visible structured observation cannot diverge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensAgentProjection {
    canonical: CanonicalProjection,
    prompt_media: Vec<LensMediaPayload>,
}

#[derive(Serialize)]
struct LensAgentProjectionPayload {
    schema_version: u32,
    sources: Vec<LensAgentProjectionSource>,
    media: Vec<LensAgentProjectionMedia>,
    media_omissions: Vec<LensAgentProjectionMediaOmission>,
    quality: ExtractionQuality,
}

#[derive(Serialize)]
struct LensAgentProjectionSource {
    id: String,
    provenance: LensAgentProjectionSourceProvenance,
    document: Option<LensDocumentProjection>,
    quality: ExtractionQuality,
    omissions: Vec<ProjectionOmission>,
}

#[derive(Serialize)]
struct LensAgentProjectionSourceProvenance {
    application: String,
    window_title: String,
    application_id: String,
}

#[derive(Serialize)]
struct LensAgentProjectionMedia {
    id: String,
    source_id: String,
    uri: String,
    scope: LensMediaScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_node_id: Option<String>,
    source_bounds: Bounds,
    captured_bounds: Bounds,
    coverage: LensMediaCoverage,
    coordinate_space: ProjectionCoordinateSpace,
    mime_type: String,
    pixel_width: usize,
    pixel_height: usize,
    encoded_bytes: usize,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProjectionCoordinateSpace {
    TargetLogical,
    OriginalCapturePixels,
    WindowRelativePoints,
}

#[derive(Serialize)]
struct LensAgentProjectionMediaOmission {
    source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attachment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_node_id: Option<String>,
    reason: LensMediaOmissionReason,
    omitted_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_order: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_order: Option<usize>,
    detail: String,
}

impl LensAgentProjection {
    pub fn from_input(
        input: &LensInput,
        target_set: &LensTargetSet,
        media: &[LensMediaPayload],
    ) -> Result<Self, LensAgentProjectionError> {
        if target_set.validate().is_err() {
            return Err(LensAgentProjectionError::TargetSetMismatch);
        }
        let mut payloads = BTreeMap::new();
        for payload in media {
            if payloads
                .insert(payload.attachment_id.as_str(), payload)
                .is_some()
            {
                return Err(LensAgentProjectionError::DuplicateMediaPayload {
                    attachment_id: payload.attachment_id.clone(),
                });
            }
        }

        let targets = target_set
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| (target.id.as_str(), (index, target)))
            .collect::<BTreeMap<_, _>>();
        if targets.len() != target_set.targets.len() || input.sources.len() != targets.len() {
            return Err(LensAgentProjectionError::TargetSetMismatch);
        }

        let mut node_ids = BTreeMap::new();
        let mut source_ids = BTreeMap::new();
        let mut sources = Vec::with_capacity(input.sources.len());
        for source in &input.sources {
            let (source_index, target) =
                targets.get(source.target_id.as_str()).ok_or_else(|| {
                    LensAgentProjectionError::UnknownTarget {
                        target_id: source.target_id.clone(),
                    }
                })?;
            if source.source.receipt != target.identity.receipt
                || source.source.application_id != target.facts.application_id
            {
                return Err(LensAgentProjectionError::TargetSetMismatch);
            }
            let source_id = format!("source-{source_index}");
            if source_ids
                .insert(source.target_id.clone(), source_id.clone())
                .is_some()
            {
                return Err(LensAgentProjectionError::DuplicateTarget {
                    target_id: source.target_id.clone(),
                });
            }
            let document = source
                .document
                .as_ref()
                .map(|document| {
                    normalize_projection_document(
                        document,
                        &source.target_id,
                        &source_id,
                        &mut node_ids,
                    )
                })
                .transpose()?;
            sources.push(LensAgentProjectionSource {
                id: source_id,
                provenance: LensAgentProjectionSourceProvenance {
                    application: source.source.application.clone(),
                    window_title: source.source.window_title.clone(),
                    application_id: source.source.application_id.clone(),
                },
                document,
                quality: source.quality,
                omissions: source.omissions.clone(),
            });
        }
        sources.sort_by(|left, right| left.id.cmp(&right.id));

        let expected = input
            .media
            .iter()
            .map(|attachment| attachment.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut media_ids = BTreeMap::new();
        let mut per_source_media_order = BTreeMap::<&str, usize>::new();
        for attachment in &input.media {
            let source_id = source_ids.get(&attachment.target_id).ok_or_else(|| {
                LensAgentProjectionError::UnknownTarget {
                    target_id: attachment.target_id.clone(),
                }
            })?;
            let order = per_source_media_order
                .entry(attachment.target_id.as_str())
                .or_default();
            let projection_id = format!("{source_id}/media-{order}");
            *order += 1;
            if media_ids
                .insert(attachment.id.clone(), projection_id)
                .is_some()
            {
                return Err(LensAgentProjectionError::DuplicateMediaAttachment {
                    attachment_id: attachment.id.clone(),
                });
            }
        }

        for source in &mut sources {
            if let Some(document) = source.document.as_mut() {
                for node in &mut document.nodes {
                    node.media_refs = node
                        .media_refs
                        .iter()
                        .map(|id| {
                            media_ids.get(id).cloned().ok_or_else(|| {
                                LensAgentProjectionError::UnknownMediaReference {
                                    attachment_id: id.clone(),
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                }
            }
        }

        let mut normalized_media = Vec::with_capacity(input.media.len());
        let mut prompt_media = Vec::with_capacity(input.media.len());
        for attachment in &input.media {
            let payload = payloads.get(attachment.id.as_str()).ok_or_else(|| {
                LensAgentProjectionError::MissingMediaPayload {
                    attachment_id: attachment.id.clone(),
                }
            })?;
            if payload.mime_type != attachment.mime_type || payload.uri != attachment.uri {
                return Err(LensAgentProjectionError::MediaMetadataMismatch {
                    attachment_id: attachment.id.clone(),
                });
            }
            let decoded = BASE64_STANDARD.decode(&payload.data).map_err(|source| {
                LensAgentProjectionError::InvalidMediaPayload {
                    attachment_id: attachment.id.clone(),
                    source,
                }
            })?;
            let source_id = source_ids
                .get(&attachment.target_id)
                .expect("attachment target was validated above");
            let (_, target) = targets
                .get(attachment.target_id.as_str())
                .expect("attachment target was validated above");
            let projection_id = media_ids
                .get(&attachment.id)
                .expect("media identities were built from every attachment")
                .clone();
            let projection_uri = format!("lens://projection/{projection_id}");
            let source_node_id = attachment
                .source_node_id
                .as_ref()
                .map(|id| {
                    node_ids
                        .get(&(attachment.target_id.clone(), id.clone()))
                        .cloned()
                        .ok_or_else(|| LensAgentProjectionError::UnknownNodeReference {
                            node_id: id.clone(),
                        })
                })
                .transpose()?;
            let (source_bounds, captured_bounds, coordinate_space) =
                acquired_media_bounds(attachment, target)?;
            normalized_media.push(LensAgentProjectionMedia {
                id: projection_id.clone(),
                source_id: source_id.clone(),
                uri: projection_uri.clone(),
                scope: attachment.scope,
                source_node_id,
                source_bounds,
                captured_bounds,
                coverage: attachment.coverage,
                coordinate_space,
                mime_type: attachment.mime_type.clone(),
                pixel_width: attachment.pixel_width,
                pixel_height: attachment.pixel_height,
                encoded_bytes: attachment.encoded_bytes,
                sha256: hex_digest(&Sha256::digest(decoded)),
            });
            prompt_media.push(LensMediaPayload {
                attachment_id: projection_id,
                uri: projection_uri,
                mime_type: payload.mime_type.clone(),
                data: payload.data.clone(),
            });
        }
        if let Some(unexpected) = payloads.keys().find(|id| !expected.contains(**id)) {
            return Err(LensAgentProjectionError::UnexpectedMediaPayload {
                attachment_id: (*unexpected).to_owned(),
            });
        }

        let mut replacements = Vec::new();
        replacements.push((input.context_id.to_string(), "context".to_string()));
        replacements.extend(
            source_ids
                .iter()
                .map(|(original, normalized)| (original.clone(), normalized.clone())),
        );
        replacements.extend(
            media_ids
                .iter()
                .map(|(original, normalized)| (original.clone(), normalized.clone())),
        );
        replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0.len()));

        let media_omissions = input
            .media_omissions
            .iter()
            .map(|omission| {
                let source_id = source_ids.get(&omission.target_id).ok_or_else(|| {
                    LensAgentProjectionError::UnknownTarget {
                        target_id: omission.target_id.clone(),
                    }
                })?;
                Ok(LensAgentProjectionMediaOmission {
                    source_id: source_id.clone(),
                    attachment_id: omission.attachment_id.as_ref().map(|id| {
                        media_ids
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| normalize_opaque_id(id))
                    }),
                    source_node_id: omission.source_node_id.as_ref().map(|id| {
                        node_ids
                            .get(&(omission.target_id.clone(), id.clone()))
                            .cloned()
                            .unwrap_or_else(|| normalize_opaque_id(id))
                    }),
                    reason: omission.reason,
                    omitted_count: omission.omitted_count,
                    first_order: omission.first_order,
                    last_order: omission.last_order,
                    detail: normalize_detail(&omission.detail, &replacements),
                })
            })
            .collect::<Result<Vec<_>, LensAgentProjectionError>>()?;

        let canonical = CanonicalProjection::from_serializable(&LensAgentProjectionPayload {
            schema_version: 5,
            sources,
            media: normalized_media,
            media_omissions,
            quality: input.quality,
        })
        .map_err(LensAgentProjectionError::Canonicalization)?;
        Ok(Self {
            canonical,
            prompt_media,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        self.canonical.bytes()
    }

    pub fn digest(&self) -> &ProjectionDigest {
        self.canonical.digest()
    }

    pub fn projection_ref(&self, revision: NonZeroU64) -> ProjectionRef {
        self.canonical.projection_ref(revision)
    }

    pub fn json(&self) -> &str {
        // RFC 8785 output is always valid UTF-8 JSON because the input came from serde.
        std::str::from_utf8(self.canonical.bytes())
            .expect("serde_json canonicalization emits UTF-8")
    }

    pub fn prompt_media(&self) -> &[LensMediaPayload] {
        &self.prompt_media
    }
}

fn normalize_projection_document(
    document: &LensDocumentProjection,
    source_key: &str,
    source_id: &str,
    all_node_ids: &mut BTreeMap<(String, String), String>,
) -> Result<LensDocumentProjection, LensAgentProjectionError> {
    let mut local_ids = BTreeMap::new();
    for (index, node) in document.nodes.iter().enumerate() {
        let normalized = format!("{source_id}/node-{index}");
        if local_ids
            .insert(node.id.clone(), normalized.clone())
            .is_some()
            || all_node_ids
                .insert((source_key.to_string(), node.id.clone()), normalized)
                .is_some()
        {
            return Err(LensAgentProjectionError::DuplicateNode {
                node_id: node.id.clone(),
            });
        }
    }
    let nodes = document
        .nodes
        .iter()
        .map(|node| normalize_projection_node(node, &local_ids))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LensDocumentProjection { nodes })
}

fn normalize_projection_node(
    node: &LensContentNode,
    ids: &BTreeMap<String, String>,
) -> Result<LensContentNode, LensAgentProjectionError> {
    let resolve = |id: &str| {
        ids.get(id)
            .cloned()
            .ok_or_else(|| LensAgentProjectionError::UnknownNodeReference {
                node_id: id.to_string(),
            })
    };
    Ok(LensContentNode {
        id: resolve(&node.id)?,
        parent_id: node.parent_id.as_deref().map(resolve).transpose()?,
        kind: node.kind,
        source_api: node.source_api,
        node_purpose: node.node_purpose,
        native_role: node.native_role.clone(),
        native_subrole: node.native_subrole.clone(),
        title: node.title.clone(),
        value: node.value.clone(),
        description: node.description.clone(),
        media_refs: node.media_refs.clone(),
        resource_refs: node.resource_refs.clone(),
    })
}

fn acquired_media_bounds(
    attachment: &crate::lens::LensMediaAttachment,
    target: &crate::lens::LensTarget,
) -> Result<(Bounds, Bounds, ProjectionCoordinateSpace), LensAgentProjectionError> {
    let invalid = || LensAgentProjectionError::MediaMetadataMismatch {
        attachment_id: attachment.id.clone(),
    };
    let Some(pixels) = &attachment.geometry else {
        if attachment.desktop_geometry.is_some() {
            return Err(invalid());
        }
        return Ok((
            relative_bounds(attachment.source_bounds, target.facts.frame),
            relative_bounds(attachment.captured_bounds, target.facts.frame),
            ProjectionCoordinateSpace::WindowRelativePoints,
        ));
    };
    pixels.validate().map_err(|_| invalid())?;
    if pixels.attachment_id != attachment.id
        || pixels.capture.read.target.operation_id != target.identity.operation_id
        || uuid::Uuid::from(pixels.capture.read.target.receipt) != target.identity.receipt
        || pixels.encoded_extent.width as usize != attachment.pixel_width
        || pixels.encoded_extent.height as usize != attachment.pixel_height
    {
        return Err(invalid());
    }
    let crop = Rect {
        x: f64::from(pixels.crop.x),
        y: f64::from(pixels.crop.y),
        width: f64::from(pixels.crop.extent.width),
        height: f64::from(pixels.crop.extent.height),
    };
    let bounds = |rect: Rect| Bounds {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    let Some(desktop) = &attachment.desktop_geometry else {
        if attachment.scope != LensMediaScope::WindowFallback {
            return Err(invalid());
        }
        return Ok((
            Bounds {
                x: 0.0,
                y: 0.0,
                width: f64::from(pixels.original_extent.width),
                height: f64::from(pixels.original_extent.height),
            },
            bounds(crop),
            ProjectionCoordinateSpace::OriginalCapturePixels,
        ));
    };
    desktop.validate().map_err(|_| invalid())?;
    if desktop.read != pixels.capture.read {
        return Err(invalid());
    }
    let window = desktop.window.rect;
    let scale_x = window.width / f64::from(pixels.original_extent.width);
    let scale_y = window.height / f64::from(pixels.original_extent.height);
    let footprint = Rect {
        x: window.x + crop.x * scale_x,
        y: window.y + crop.y * scale_y,
        width: crop.width * scale_x,
        height: crop.height * scale_y,
    };
    let transform = |rect| {
        desktop
            .desktop_to_target
            .apply(&TaggedRect {
                read: desktop.read,
                frame: desktop.window.frame.clone(),
                rect,
            })
            .map(|tagged| bounds(tagged.rect))
            .map_err(|_| invalid())
    };
    let source = attachment.source_bounds;
    Ok((
        transform(Rect {
            x: source.x,
            y: source.y,
            width: source.width,
            height: source.height,
        })?,
        transform(footprint)?,
        ProjectionCoordinateSpace::TargetLogical,
    ))
}

fn relative_bounds(bounds: Bounds, window: Bounds) -> Bounds {
    Bounds {
        x: bounds.x - window.x,
        y: bounds.y - window.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn normalize_detail(detail: &str, replacements: &[(String, String)]) -> String {
    replacements
        .iter()
        .fold(detail.to_string(), |current, pair| {
            current.replace(&pair.0, &pair.1)
        })
}

fn normalize_opaque_id(value: &str) -> String {
    format!("opaque-{}", hex_digest(&Sha256::digest(value.as_bytes())))
}

#[derive(Debug, Error)]
pub enum LensAgentProjectionError {
    #[error("Agent projection target set does not exactly match its LensInput sources")]
    TargetSetMismatch,
    #[error("Agent projection references unknown target {target_id}")]
    UnknownTarget { target_id: String },
    #[error("Agent projection contains duplicate target {target_id}")]
    DuplicateTarget { target_id: String },
    #[error("Agent projection contains duplicate node {node_id}")]
    DuplicateNode { node_id: String },
    #[error("Agent projection references unknown node {node_id}")]
    UnknownNodeReference { node_id: String },
    #[error("Agent projection contains duplicate media attachment {attachment_id}")]
    DuplicateMediaAttachment { attachment_id: String },
    #[error("Agent projection references unknown media attachment {attachment_id}")]
    UnknownMediaReference { attachment_id: String },
    #[error("Agent projection has duplicate media payload {attachment_id}")]
    DuplicateMediaPayload { attachment_id: String },
    #[error("Agent projection is missing media payload {attachment_id}")]
    MissingMediaPayload { attachment_id: String },
    #[error("Agent projection contains unexpected media payload {attachment_id}")]
    UnexpectedMediaPayload { attachment_id: String },
    #[error("Agent projection media metadata does not match attachment {attachment_id}")]
    MediaMetadataMismatch { attachment_id: String },
    #[error("Agent projection media payload {attachment_id} is not valid base64: {source}")]
    InvalidMediaPayload {
        attachment_id: String,
        #[source]
        source: base64::DecodeError,
    },
    #[error("Agent projection canonicalization failed: {0}")]
    Canonicalization(#[source] CanonicalProjectionError),
}

#[derive(Debug, Error)]
pub enum CanonicalProjectionError {
    #[error("projection serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    #[error("RFC 8785 canonicalization failed: {0}")]
    Canonicalization(#[source] serde_json::Error),
    #[error("projection integer at {path} is outside the exact I-JSON range: {value}")]
    UnsafeInteger { path: String, value: String },
    #[error(
        "projection Serialize implementation produced different semantic values across passes"
    )]
    NonDeterministicSerialization,
}

fn validate_i_json_numbers(
    value: &serde_json::Value,
    pointer: &str,
) -> Result<(), CanonicalProjectionError> {
    match value {
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_i_json_numbers(value, &format!("{pointer}/{index}"))?;
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                validate_i_json_numbers(value, &format!("{pointer}/{escaped}"))?;
            }
        }
        serde_json::Value::Number(number) => {
            let unsafe_integer = number
                .as_u64()
                .is_some_and(|value| value > MAX_EXACT_I_JSON_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|value| value.unsigned_abs() > MAX_EXACT_I_JSON_INTEGER);
            if unsafe_integer {
                return Err(CanonicalProjectionError::UnsafeInteger {
                    path: if pointer.is_empty() {
                        "/".into()
                    } else {
                        pointer.into()
                    },
                    value: number.to_string(),
                });
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {}
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// A validated lowercase hexadecimal SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectionDigest(String);

impl ProjectionDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectionDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ProjectionDigest {
    type Err = ProjectionDigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProjectionDigestError);
        }
        Ok(Self(value.into()))
    }
}

impl<'de> Deserialize<'de> for ProjectionDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(|_| de::Error::custom("expected a 64-character lowercase SHA-256 digest"))
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("expected a 64-character lowercase SHA-256 digest")]
pub struct ProjectionDigestError;

/// An Agent-visible semantic projection identity. Revision zero is not representable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRef {
    pub revision: NonZeroU64,
    pub digest: ProjectionDigest,
}

impl ProjectionRef {
    pub fn new(revision: NonZeroU64, digest: ProjectionDigest) -> Self {
        Self { revision, digest }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lens::{
            target_id, LensCoordinateSpace, LensInputSource, LensMediaAttachment,
            LensMediaCoverage, LensNodeKind, LensSource, LENS_INPUT_SCHEMA_VERSION,
        },
        model::{NodePurpose, SelectedWindow, SourceApi, WindowIdentity, WindowObservableFacts},
    };
    use serde_json::json;

    use uuid::Uuid;

    fn agent_projection_fixture(
        context_id: Uuid,
        context_revision: u64,
        source_revision: u64,
        window_id: u32,
        window_origin: (f64, f64),
        text: &str,
        media_data: &str,
    ) -> (LensInput, LensTargetSet, Vec<LensMediaPayload>) {
        let window = SelectedWindow {
            identity: WindowIdentity {
                operation_id: context_id,
                receipt: Uuid::from_u128(u128::from(window_id)),
                selection_ordinal: 1,
            },
            facts: WindowObservableFacts {
                application_id: "example.browser".into(),
                title: "Document".into(),
                application_name: "Browser".into(),
                frame: Bounds {
                    x: window_origin.0,
                    y: window_origin.1,
                    width: 400.0,
                    height: 300.0,
                },
            },
        };
        let target_id = target_id(&window);
        let attachment_id = format!("media-window-{window_id}-node-000001");
        let uri = format!("lens://context/{context_id}/{context_revision}/media/{attachment_id}");
        let input = LensInput {
            schema_version: LENS_INPUT_SCHEMA_VERSION,
            context_id,
            context_revision,
            sources: vec![LensInputSource {
                source_id: format!("{target_id}:accessibility"),
                target_id: target_id.clone(),
                source_revision,
                source: LensSource {
                    application: "Browser".into(),
                    window_title: "Document".into(),
                    application_id: "example.browser".into(),
                    receipt: window.identity.receipt,
                },
                document: Some(LensDocumentProjection {
                    nodes: vec![
                        LensContentNode {
                            id: "node-000000".into(),
                            parent_id: None,
                            kind: LensNodeKind::Region,
                            source_api: SourceApi::MacosAx,
                            node_purpose: NodePurpose::WindowChrome,
                            native_role: Some("AXWindow".into()),
                            native_subrole: None,
                            title: Some("Document".into()),
                            value: None,
                            description: None,
                            media_refs: vec![],
                            resource_refs: vec![],
                        },
                        LensContentNode {
                            id: "node-000001".into(),
                            parent_id: Some("node-000000".into()),
                            kind: LensNodeKind::Image,
                            source_api: SourceApi::MacosAx,
                            node_purpose: NodePurpose::Content,
                            native_role: Some("AXImage".into()),
                            native_subrole: None,
                            title: None,
                            value: Some(text.into()),
                            description: Some("Chart".into()),
                            media_refs: vec![attachment_id.clone()],
                            resource_refs: vec![],
                        },
                    ],
                }),
                quality: ExtractionQuality::Full,
                omissions: vec![],
            }],
            media: vec![LensMediaAttachment {
                geometry: None,
                desktop_geometry: None,
                id: attachment_id.clone(),
                target_id,
                uri: uri.clone(),
                scope: LensMediaScope::AccessibilityElementRegion,
                source_node_id: Some("node-000001".into()),
                source_bounds: Bounds {
                    x: window_origin.0 + 10.0,
                    y: window_origin.1 + 20.0,
                    width: 100.0,
                    height: 80.0,
                },
                captured_bounds: Bounds {
                    x: window_origin.0 + 10.0,
                    y: window_origin.1 + 20.0,
                    width: 100.0,
                    height: 80.0,
                },
                coverage: LensMediaCoverage::FullRegion,
                coordinate_space: LensCoordinateSpace::ScreenPoints,
                mime_type: "image/png".into(),
                pixel_width: 200,
                pixel_height: 160,
                encoded_bytes: BASE64_STANDARD
                    .decode(media_data)
                    .expect("fixture base64")
                    .len(),
            }],
            media_omissions: vec![],
            quality: ExtractionQuality::Full,
        };
        let target_set = LensTargetSet::try_new(context_id, vec![window]).expect("target set");
        let payloads = vec![LensMediaPayload {
            attachment_id,
            uri,
            mime_type: "image/png".into(),
            data: media_data.into(),
        }];
        (input, target_set, payloads)
    }

    #[test]
    fn projection_rejects_source_receipt_not_owned_by_its_target() {
        let (mut input, targets, media) =
            agent_projection_fixture(Uuid::from_u128(1), 1, 1, 7, (0.0, 0.0), "Text", "aGVsbG8=");
        input.sources[0].source.receipt = Uuid::from_u128(99);
        assert!(matches!(
            LensAgentProjection::from_input(&input, &targets, &media),
            Err(LensAgentProjectionError::TargetSetMismatch)
        ));
    }

    #[test]
    fn acquired_pixels_survive_latest_movement_and_hide_capture_identity() {
        use crate::geometry::*;
        let (mut input, mut targets, media) = agent_projection_fixture(
            Uuid::from_u128(1),
            1,
            1,
            7,
            (100.0, 200.0),
            "Content",
            "aGVsbG8=",
        );
        let attachment = &mut input.media[0];
        let read = TargetReadKey {
            target: TargetAuthority {
                operation_id: Uuid::from_u128(1),
                receipt: Uuid::from_u128(7).try_into().unwrap(),
            },
            sequence: ReadSequence::FIRST,
        };
        let capture = CaptureKey {
            read,
            capture_id: Uuid::from_u128(100),
        };
        let original = CoordinateFrame::OriginalCapturePixels { capture };
        let cropped = CoordinateFrame::CroppedAttachmentPixels {
            capture,
            attachment_id: attachment.id.clone(),
        };
        let encoded = CoordinateFrame::EncodedAttachmentPixels {
            capture,
            attachment_id: attachment.id.clone(),
        };
        attachment.geometry = Some(AttachmentGeometry {
            capture,
            attachment_id: attachment.id.clone(),
            original_extent: PixelExtent {
                width: 800,
                height: 600,
            },
            crop: PixelCrop {
                x: 19,
                y: 39,
                extent: PixelExtent {
                    width: 202,
                    height: 162,
                },
            },
            encoded_extent: PixelExtent {
                width: attachment.pixel_width as u32,
                height: attachment.pixel_height as u32,
            },
            original_to_crop: AxisAlignedTransform {
                read,
                source: original,
                destination: cropped.clone(),
                scale_x: 1.0,
                scale_y: 1.0,
                translate_x: -19.0,
                translate_y: -39.0,
            },
            crop_to_encoded: AxisAlignedTransform {
                read,
                source: cropped,
                destination: encoded,
                scale_x: attachment.pixel_width as f64 / 202.0,
                scale_y: attachment.pixel_height as f64 / 162.0,
                translate_x: 0.0,
                translate_y: 0.0,
            },
        });
        attachment.desktop_geometry = Some(ReadGeometryDescriptor {
            read,
            window: TaggedRect {
                read,
                frame: CoordinateFrame::MacosDesktopPoints,
                rect: Rect {
                    x: 100.0,
                    y: 200.0,
                    width: 400.0,
                    height: 300.0,
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
                translate_x: -100.0,
                translate_y: -200.0,
            },
        });
        let first = LensAgentProjection::from_input(&input, &targets, &media).unwrap();
        let json: serde_json::Value = serde_json::from_str(first.json()).unwrap();
        assert_eq!(json["media"][0]["captured_bounds"]["x"], 9.5);
        assert_eq!(json["media"][0]["captured_bounds"]["width"], 101.0);
        assert_eq!(json["media"][0]["coordinate_space"], "target_logical");
        targets.targets[0].facts.frame.x = 900.0;
        // Change every private capture key consistently, without altering pixels.
        let mut geometry = serde_json::to_value(input.media[0].geometry.as_ref().unwrap()).unwrap();
        fn change_capture(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    if let Some(id) = fields.get_mut("capture_id") {
                        *id = serde_json::json!(Uuid::from_u128(101));
                    }
                    for child in fields.values_mut() {
                        change_capture(child);
                    }
                }
                serde_json::Value::Array(items) => {
                    for child in items {
                        change_capture(child);
                    }
                }
                _ => {}
            }
        }
        change_capture(&mut geometry);
        input.media[0].geometry = Some(serde_json::from_value(geometry).unwrap());
        assert_eq!(
            first.bytes(),
            LensAgentProjection::from_input(&input, &targets, &media)
                .unwrap()
                .bytes()
        );
        input.media[0].desktop_geometry = None;
        assert!(LensAgentProjection::from_input(&input, &targets, &media).is_err());
        input.media[0].scope = LensMediaScope::WindowFallback;
        let fallback = LensAgentProjection::from_input(&input, &targets, &media).unwrap();
        let json: serde_json::Value = serde_json::from_str(fallback.json()).unwrap();
        assert_eq!(
            json["media"][0]["coordinate_space"],
            "original_capture_pixels"
        );
        assert_eq!(json["media"][0]["source_bounds"]["width"], 800.0);
        assert_eq!(json["media"][0]["captured_bounds"]["x"], 19.0);
    }

    #[test]
    fn agent_projection_excludes_transport_revisions_ids_and_screen_origin() {
        let first_context = Uuid::from_u128(0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);
        let second_context = Uuid::from_u128(0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
        let (first_input, first_targets, first_media) = agent_projection_fixture(
            first_context,
            1,
            1,
            7,
            (100.0, 200.0),
            "Revenue rose",
            "aGVsbG8=",
        );
        let (second_input, second_targets, second_media) = agent_projection_fixture(
            second_context,
            19,
            8,
            99,
            (900.0, 1200.0),
            "Revenue rose",
            "aGVsbG8=",
        );

        let first = LensAgentProjection::from_input(&first_input, &first_targets, &first_media)
            .expect("first projection");
        let second = LensAgentProjection::from_input(&second_input, &second_targets, &second_media)
            .expect("second projection");

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.digest(), second.digest());
        let encoded: serde_json::Value = serde_json::from_str(first.json()).unwrap();
        assert_eq!(encoded["schema_version"], 5);
        assert_eq!(encoded["media"][0]["scope"], "accessibility_element_region");
        assert!(!first.json().contains(&first_context.to_string()));
        assert!(!first.json().contains("node-000001"));
        assert!(!first.json().contains("media-window-7"));
        assert!(first.json().contains("window_relative_points"));
        assert_eq!(first.prompt_media()[0].attachment_id, "source-0/media-0");
        assert_eq!(
            first.prompt_media()[0].uri,
            "lens://projection/source-0/media-0"
        );
    }

    #[test]
    fn agent_projection_digest_covers_text_and_exact_media_bytes() {
        let context = Uuid::from_u128(1);
        let (input, targets, media) =
            agent_projection_fixture(context, 1, 1, 7, (0.0, 0.0), "Revenue rose", "aGVsbG8=");
        let (changed_text, _, changed_text_media) =
            agent_projection_fixture(context, 2, 2, 7, (0.0, 0.0), "Revenue fell", "aGVsbG8=");
        let mut changed_media = media.clone();
        changed_media[0].data = "d29ybGQ=".into();

        let base = LensAgentProjection::from_input(&input, &targets, &media).expect("base");
        let text = LensAgentProjection::from_input(&changed_text, &targets, &changed_text_media)
            .expect("text change");
        let image = LensAgentProjection::from_input(&input, &targets, &changed_media)
            .expect("media change");

        assert_ne!(base.digest(), text.digest());
        assert_ne!(base.digest(), image.digest());
    }

    #[test]
    fn title_only_refresh_changes_the_agent_projection_without_retargeting() {
        let context = Uuid::from_u128(2);
        let (input, targets, media) =
            agent_projection_fixture(context, 1, 1, 7, (0.0, 0.0), "Revenue rose", "aGVsbG8=");
        let base = LensAgentProjection::from_input(&input, &targets, &media).expect("base");
        let mut renamed_input = input.clone();
        renamed_input.context_revision = 2;
        renamed_input.sources[0].source_revision = 2;
        renamed_input.sources[0].source.window_title = "Renamed document".into();
        let target_id = targets.targets[0].id.clone();
        let mut renamed_facts = targets.targets[0].facts.clone();
        renamed_facts.title = "Renamed document".into();
        let renamed_targets = targets
            .refresh_observable_facts(2, BTreeMap::from([(target_id, renamed_facts)]))
            .expect("renamed facts");
        let renamed = LensAgentProjection::from_input(&renamed_input, &renamed_targets, &media)
            .expect("renamed projection");

        assert!(targets.has_same_identity(&renamed_targets));
        assert_eq!(renamed_targets.targets[0].facts_revision, 2);
        assert_ne!(base.digest(), renamed.digest());
        assert!(renamed.json().contains("Renamed document"));
    }

    #[test]
    fn frame_only_resize_advances_facts_but_preserves_the_semantic_digest() {
        let context = Uuid::from_u128(3);
        let (input, targets, media) =
            agent_projection_fixture(context, 1, 1, 7, (0.0, 0.0), "Revenue rose", "aGVsbG8=");
        let base = LensAgentProjection::from_input(&input, &targets, &media).expect("base");
        let target_id = targets.targets[0].id.clone();
        let mut resized_facts = targets.targets[0].facts.clone();
        resized_facts.frame.width = 640.0;
        resized_facts.frame.height = 360.0;
        let resized_targets = targets
            .refresh_observable_facts(2, BTreeMap::from([(target_id, resized_facts)]))
            .expect("resized facts");
        let resized = LensAgentProjection::from_input(&input, &resized_targets, &media)
            .expect("resized projection");

        assert!(targets.has_same_identity(&resized_targets));
        assert_eq!(resized_targets.targets[0].facts_revision, 2);
        assert_eq!(resized_targets.targets[0].facts.frame.width, 640.0);
        assert_eq!(base.bytes(), resized.bytes());
        assert_eq!(base.digest(), resized.digest());
    }

    #[test]
    fn moving_the_same_window_and_media_preserves_window_relative_projection() {
        let context = Uuid::from_u128(4);
        let (first_input, first_targets, first_media) =
            agent_projection_fixture(context, 1, 1, 7, (100.0, 200.0), "Revenue rose", "aGVsbG8=");
        let (moved_input, moved_targets, moved_media) = agent_projection_fixture(
            context,
            2,
            2,
            7,
            (900.0, 1200.0),
            "Revenue rose",
            "aGVsbG8=",
        );
        let first = LensAgentProjection::from_input(&first_input, &first_targets, &first_media)
            .expect("first projection");
        let moved = LensAgentProjection::from_input(&moved_input, &moved_targets, &moved_media)
            .expect("moved projection");

        assert!(first_targets.has_same_identity(&moved_targets));
        assert_ne!(
            first_targets.targets[0].facts.frame,
            moved_targets.targets[0].facts.frame
        );
        assert_eq!(first.bytes(), moved.bytes());
        assert_eq!(first.digest(), moved.digest());
    }

    #[test]
    fn agent_projection_rejects_inexact_media_bundle() {
        let context = Uuid::from_u128(1);
        let (input, targets, mut media) =
            agent_projection_fixture(context, 1, 1, 7, (0.0, 0.0), "Revenue rose", "aGVsbG8=");
        media[0].uri = "lens://wrong".into();
        assert!(matches!(
            LensAgentProjection::from_input(&input, &targets, &media),
            Err(LensAgentProjectionError::MediaMetadataMismatch { .. })
        ));
    }

    #[test]
    fn canonicalization_matches_rfc_8785_section_3_2_2() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{
              "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001],
              "string": "€$\u000f\nA'B\"\\\\\"/",
              "literals": [null, true, false]
            }"#,
        )
        .expect("RFC example is valid JSON");
        let canonical =
            CanonicalProjection::from_serializable(&value).expect("RFC example canonicalizes");

        assert_eq!(
            canonical.bytes(),
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"string":"€$\u000f\nA'B\"\\\\\"/"}"#.as_bytes()
        );
    }

    #[test]
    fn projection_golden_bytes_and_digest_ignore_object_insertion_order() {
        let first = json!({
            "sources": [{
                "source": "source-0",
                "quality": "full",
                "nodes": [
                    {"kind": "heading", "id": "node-0", "title": "Résumé"},
                    {"value": "\u{6c7a}\u{5b9a}\u{8ad6}\u{7684}\u{306a}\u{7ffb}\u{8a33}", "kind": "paragraph", "id": "node-1"}
                ],
                "provenance": {"windowTitle": "Article", "bundleIdentifier": "com.example.Reader", "application": "Reader"}
            }],
            "schemaVersion": 1,
            "omissions": []
        });
        let reordered: serde_json::Value = serde_json::from_str(
            r#"{"omissions":[],"schemaVersion":1,"sources":[{"nodes":[{"title":"Résumé","id":"node-0","kind":"heading"},{"id":"node-1","kind":"paragraph","value":"\u6c7a\u5b9a\u8ad6\u7684\u306a\u7ffb\u8a33"}],"provenance":{"application":"Reader","bundleIdentifier":"com.example.Reader","windowTitle":"Article"},"quality":"full","source":"source-0"}]}"#,
        )
        .expect("fixture JSON");

        let first = CanonicalProjection::from_serializable(&first).expect("canonical projection");
        let reordered =
            CanonicalProjection::from_serializable(&reordered).expect("canonical projection");

        assert_eq!(first.bytes(), reordered.bytes());
        assert_eq!(
            std::str::from_utf8(first.bytes()).expect("UTF-8"),
            concat!(
                r#"{"omissions":[],"schemaVersion":1,"sources":[{"nodes":[{"id":"node-0","kind":"heading","title":"Résumé"},{"id":"node-1","kind":"paragraph","value":""#,
                "\u{6c7a}\u{5b9a}\u{8ad6}\u{7684}\u{306a}\u{7ffb}\u{8a33}",
                r#""}],"provenance":{"application":"Reader","bundleIdentifier":"com.example.Reader","windowTitle":"Article"},"quality":"full","source":"source-0"}]}"#
            )
        );
        assert_eq!(
            first.digest().as_str(),
            "dccc2adc927d65da9247f586c8606451b66f74736170b4805d09b540da12efd2"
        );
    }

    #[test]
    fn canonicalization_rejects_non_finite_and_unsafe_integer_values() {
        assert!(matches!(
            CanonicalProjection::from_serializable(&f64::NAN),
            Err(CanonicalProjectionError::Canonicalization(_))
        ));
        assert!(matches!(
            CanonicalProjection::from_serializable(&(MAX_EXACT_I_JSON_INTEGER + 1)),
            Err(CanonicalProjectionError::UnsafeInteger { .. })
        ));
    }
}
