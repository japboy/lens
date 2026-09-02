//! Deterministic, platform-independent contracts for the planned live Lens pipeline.
//!
//! This module intentionally owns only identities and finite coordination state. Native API calls,
//! semantic projection construction, ACP transport, and UI publication remain separate layers.

use crate::{
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
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    num::{NonZeroU64, NonZeroUsize},
    str::FromStr,
};
use thiserror::Error;
use uuid::Uuid;

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
    bundle_id: String,
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
    coordinate_space: &'static str,
    mime_type: String,
    pixel_width: usize,
    pixel_height: usize,
    encoded_bytes: usize,
    sha256: String,
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
            let (source_index, _) = targets.get(source.target_id.as_str()).ok_or_else(|| {
                LensAgentProjectionError::UnknownTarget {
                    target_id: source.target_id.clone(),
                }
            })?;
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
                    bundle_id: source.source.bundle_id.clone(),
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
            normalized_media.push(LensAgentProjectionMedia {
                id: projection_id.clone(),
                source_id: source_id.clone(),
                uri: projection_uri.clone(),
                scope: attachment.scope,
                source_node_id,
                source_bounds: relative_bounds(attachment.source_bounds, target.facts.frame),
                captured_bounds: relative_bounds(attachment.captured_bounds, target.facts.frame),
                coverage: attachment.coverage,
                coordinate_space: "window_relative_points",
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
            schema_version: 1,
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
        role: node.role.clone(),
        subrole: node.subrole.clone(),
        title: node.title.clone(),
        value: node.value.clone(),
        description: node.description.clone(),
        media_refs: node.media_refs.clone(),
        resource_refs: node.resource_refs.clone(),
    })
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

/// The projection known to have been applied by the current ACP session epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentProjectionCursor {
    None,
    Applied { projection: ProjectionRef },
}

/// Local authority attached to one serial ACP prompt and all of its updates/completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentTurnKey {
    operation_id: Uuid,
    context_id: Uuid,
    session_epoch: NonZeroU64,
    turn_id: Uuid,
    base_projection: AgentProjectionCursor,
    target_projection: ProjectionRef,
}

impl AgentTurnKey {
    pub fn try_new(
        operation_id: Uuid,
        context_id: Uuid,
        session_epoch: NonZeroU64,
        turn_id: Uuid,
        base_projection: AgentProjectionCursor,
        target_projection: ProjectionRef,
    ) -> Result<Self, AgentTurnKeyError> {
        if let AgentProjectionCursor::Applied { projection } = &base_projection {
            if projection.revision >= target_projection.revision {
                return Err(AgentTurnKeyError::NonAdvancingRevision);
            }
            if projection.digest == target_projection.digest {
                return Err(AgentTurnKeyError::UnchangedDigest);
            }
        }
        Ok(Self {
            operation_id,
            context_id,
            session_epoch,
            turn_id,
            base_projection,
            target_projection,
        })
    }

    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(&self) -> Uuid {
        self.context_id
    }

    pub fn session_epoch(&self) -> NonZeroU64 {
        self.session_epoch
    }

    pub fn turn_id(&self) -> Uuid {
        self.turn_id
    }

    pub fn base_projection(&self) -> &AgentProjectionCursor {
        &self.base_projection
    }

    pub fn target_projection(&self) -> &ProjectionRef {
        &self.target_projection
    }

    /// Classifies whether this completed turn may publish a candidate for the current state.
    pub fn candidate_authority(
        &self,
        completed_turn: &AgentTurnKey,
        operation_id: Uuid,
        context_id: Uuid,
        session_epoch: NonZeroU64,
        latest_projection: &ProjectionRef,
    ) -> CandidateAuthority {
        if self != completed_turn {
            CandidateAuthority::TurnKeyMismatch
        } else if self.operation_id != operation_id {
            CandidateAuthority::OperationMismatch
        } else if self.context_id != context_id {
            CandidateAuthority::ContextMismatch
        } else if self.session_epoch != session_epoch {
            CandidateAuthority::EpochMismatch
        } else if &self.target_projection != latest_projection {
            CandidateAuthority::ProjectionMismatch
        } else {
            CandidateAuthority::Authoritative
        }
    }

    /// Advances the conversation cursor after a definitive non-cancelled response.
    ///
    /// Candidate display validation is deliberately not an input: the Agent may have applied the
    /// projection even when its representation is not displayable.
    pub fn applied_cursor_after_definitive_response(&self) -> AgentProjectionCursor {
        AgentProjectionCursor::Applied {
            projection: self.target_projection.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum AgentTurnKeyError {
    #[error("an Agent delta turn must advance the projection revision")]
    NonAdvancingRevision,
    #[error("an Agent delta turn cannot advance revision while retaining the same digest")]
    UnchangedDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAuthority {
    Authoritative,
    TurnKeyMismatch,
    OperationMismatch,
    ContextMismatch,
    EpochMismatch,
    ProjectionMismatch,
}

/// User-controlled lifecycle for one fixed source observation registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationLifecycle {
    Watching,
    Paused,
    Stopped,
}

/// Capacity-one source-local invalidation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRefreshState {
    Clean,
    Coalescing,
    Refreshing,
    DirtyWhileRefreshing,
}

/// Immutable operation/context/source scope for one native registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceRegistrationAuthority {
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
}

impl SourceRegistrationAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid, source_registration_id: Uuid) -> Self {
        Self {
            operation_id,
            context_id,
            source_registration_id,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }

    pub fn source_registration_id(self) -> Uuid {
        self.source_registration_id
    }
}

/// Authority copied into one native registration callback context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceObserverToken {
    authority: SourceRegistrationAuthority,
    observer_epoch: NonZeroU64,
}

impl SourceObserverToken {
    pub fn authority(self) -> SourceRegistrationAuthority {
        self.authority
    }

    pub fn observer_epoch(self) -> NonZeroU64 {
        self.observer_epoch
    }
}

/// Every event that may change one source's observation coordination state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObservationEvent {
    Notification { observer: SourceObserverToken },
    CoalescingElapsed { token: SourceWorkToken },
    RefreshCompleted { token: SourceWorkToken },
    Pause,
    Resume,
    Stop,
}

/// Authority for one coalescing timer and the refresh it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceWorkToken {
    observer: SourceObserverToken,
    work_generation: NonZeroU64,
}

impl SourceWorkToken {
    pub fn observer(self) -> SourceObserverToken {
        self.observer
    }

    pub fn authority(self) -> SourceRegistrationAuthority {
        self.observer.authority
    }

    pub fn observer_epoch(self) -> NonZeroU64 {
        self.observer.observer_epoch
    }

    pub fn work_generation(self) -> NonZeroU64 {
        self.work_generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationRejectionReason {
    NotWatching {
        lifecycle: ObservationLifecycle,
    },
    EpochMismatch {
        expected: NonZeroU64,
        received: NonZeroU64,
    },
    RegistrationMismatch {
        expected: SourceRegistrationAuthority,
        received: SourceRegistrationAuthority,
    },
    WorkTokenMismatch {
        expected: Option<SourceWorkToken>,
        received: SourceWorkToken,
    },
    CoalescingStateMismatch {
        state: SourceRefreshState,
    },
    RefreshCompletionStateMismatch {
        state: SourceRefreshState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExhaustedCoordinationCounter {
    ObserverEpoch,
    WorkGeneration,
}

/// Declarative effect that the native/async integration layer must perform, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceObservationEffect {
    CoalescingStarted {
        token: SourceWorkToken,
    },
    InvalidationCoalesced {
        token: SourceWorkToken,
    },
    DirtyLatched {
        active: SourceWorkToken,
    },
    DirtyAlreadyLatched {
        active: SourceWorkToken,
    },
    RefreshStarted {
        token: SourceWorkToken,
    },
    RefreshSettled {
        token: SourceWorkToken,
    },
    FollowUpCoalescingStarted {
        token: SourceWorkToken,
    },
    Paused {
        observer: SourceObserverToken,
    },
    AlreadyPaused,
    ResumedWithFullRefresh {
        token: SourceWorkToken,
    },
    AlreadyWatching,
    Stopped,
    AlreadyStopped,
    StoppedAtGenerationLimit {
        counter: ExhaustedCoordinationCounter,
    },
    Rejected {
        reason: ObservationRejectionReason,
    },
}

/// Pure state machine for callback admission, capacity-one coalescing, and Pause/Resume epochs.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceObservationCoordinator {
    authority: SourceRegistrationAuthority,
    lifecycle: ObservationLifecycle,
    observer_epoch: NonZeroU64,
    last_work_generation: u64,
    active_work: Option<SourceWorkToken>,
    refresh: SourceRefreshState,
}

impl SourceObservationCoordinator {
    pub fn new(authority: SourceRegistrationAuthority, observer_epoch: NonZeroU64) -> Self {
        Self {
            authority,
            lifecycle: ObservationLifecycle::Watching,
            observer_epoch,
            last_work_generation: 0,
            active_work: None,
            refresh: SourceRefreshState::Clean,
        }
    }

    pub fn lifecycle(&self) -> ObservationLifecycle {
        self.lifecycle
    }

    pub fn observer_epoch(&self) -> NonZeroU64 {
        self.observer_epoch
    }

    pub fn observer_token(&self) -> SourceObserverToken {
        SourceObserverToken {
            authority: self.authority,
            observer_epoch: self.observer_epoch,
        }
    }

    pub fn refresh(&self) -> SourceRefreshState {
        self.refresh
    }

    pub fn active_work(&self) -> Option<SourceWorkToken> {
        self.active_work
    }

    pub fn apply(&mut self, event: SourceObservationEvent) -> SourceObservationEffect {
        match event {
            SourceObservationEvent::Notification { observer } => {
                if let Some(rejected) = self.reject_observer_event(observer) {
                    return rejected;
                }
                match self.refresh {
                    SourceRefreshState::Clean => {
                        let Some(token) = self.issue_work_token() else {
                            return self.fail_closed_at_generation_limit(
                                ExhaustedCoordinationCounter::WorkGeneration,
                            );
                        };
                        self.refresh = SourceRefreshState::Coalescing;
                        SourceObservationEffect::CoalescingStarted { token }
                    }
                    SourceRefreshState::Coalescing => {
                        SourceObservationEffect::InvalidationCoalesced {
                            token: self.active_work.expect("coalescing owns one work token"),
                        }
                    }
                    SourceRefreshState::Refreshing => {
                        self.refresh = SourceRefreshState::DirtyWhileRefreshing;
                        SourceObservationEffect::DirtyLatched {
                            active: self.active_work.expect("refresh owns one work token"),
                        }
                    }
                    SourceRefreshState::DirtyWhileRefreshing => {
                        SourceObservationEffect::DirtyAlreadyLatched {
                            active: self.active_work.expect("dirty refresh owns one work token"),
                        }
                    }
                }
            }
            SourceObservationEvent::CoalescingElapsed { token } => {
                if let Some(rejected) = self.reject_work_event(token) {
                    return rejected;
                }
                if self.refresh == SourceRefreshState::Coalescing {
                    self.refresh = SourceRefreshState::Refreshing;
                    SourceObservationEffect::RefreshStarted { token }
                } else {
                    SourceObservationEffect::Rejected {
                        reason: ObservationRejectionReason::CoalescingStateMismatch {
                            state: self.refresh,
                        },
                    }
                }
            }
            SourceObservationEvent::RefreshCompleted { token } => {
                if let Some(rejected) = self.reject_work_event(token) {
                    return rejected;
                }
                match self.refresh {
                    SourceRefreshState::Refreshing => {
                        self.refresh = SourceRefreshState::Clean;
                        self.active_work = None;
                        SourceObservationEffect::RefreshSettled { token }
                    }
                    SourceRefreshState::DirtyWhileRefreshing => {
                        let Some(next) = self.issue_work_token() else {
                            return self.fail_closed_at_generation_limit(
                                ExhaustedCoordinationCounter::WorkGeneration,
                            );
                        };
                        self.refresh = SourceRefreshState::Coalescing;
                        SourceObservationEffect::FollowUpCoalescingStarted { token: next }
                    }
                    SourceRefreshState::Clean | SourceRefreshState::Coalescing => {
                        SourceObservationEffect::Rejected {
                            reason: ObservationRejectionReason::RefreshCompletionStateMismatch {
                                state: self.refresh,
                            },
                        }
                    }
                }
            }
            SourceObservationEvent::Pause => self.pause(),
            SourceObservationEvent::Resume => self.resume(),
            SourceObservationEvent::Stop => self.stop(),
        }
    }

    fn reject_work_event(&self, received: SourceWorkToken) -> Option<SourceObservationEffect> {
        if let Some(rejected) = self.reject_observer_event(received.observer) {
            return Some(rejected);
        }
        if self.active_work != Some(received) {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::WorkTokenMismatch {
                    expected: self.active_work,
                    received,
                },
            })
        } else {
            None
        }
    }

    fn reject_observer_event(
        &self,
        received: SourceObserverToken,
    ) -> Option<SourceObservationEffect> {
        if self.lifecycle != ObservationLifecycle::Watching {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: self.lifecycle,
                },
            })
        } else if received.authority != self.authority {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::RegistrationMismatch {
                    expected: self.authority,
                    received: received.authority,
                },
            })
        } else if received.observer_epoch != self.observer_epoch {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::EpochMismatch {
                    expected: self.observer_epoch,
                    received: received.observer_epoch,
                },
            })
        } else {
            None
        }
    }

    fn pause(&mut self) -> SourceObservationEffect {
        match self.lifecycle {
            ObservationLifecycle::Watching => match self.advance_observer_epoch() {
                Some(_) => {
                    self.lifecycle = ObservationLifecycle::Paused;
                    self.refresh = SourceRefreshState::Clean;
                    self.active_work = None;
                    SourceObservationEffect::Paused {
                        observer: self.observer_token(),
                    }
                }
                None => self
                    .fail_closed_at_generation_limit(ExhaustedCoordinationCounter::ObserverEpoch),
            },
            ObservationLifecycle::Paused => SourceObservationEffect::AlreadyPaused,
            ObservationLifecycle::Stopped => SourceObservationEffect::AlreadyStopped,
        }
    }

    fn resume(&mut self) -> SourceObservationEffect {
        match self.lifecycle {
            ObservationLifecycle::Watching => SourceObservationEffect::AlreadyWatching,
            ObservationLifecycle::Paused => match self.advance_observer_epoch() {
                Some(_) => {
                    let Some(token) = self.issue_work_token() else {
                        return self.fail_closed_at_generation_limit(
                            ExhaustedCoordinationCounter::WorkGeneration,
                        );
                    };
                    self.lifecycle = ObservationLifecycle::Watching;
                    self.refresh = SourceRefreshState::Refreshing;
                    SourceObservationEffect::ResumedWithFullRefresh { token }
                }
                None => self
                    .fail_closed_at_generation_limit(ExhaustedCoordinationCounter::ObserverEpoch),
            },
            ObservationLifecycle::Stopped => SourceObservationEffect::AlreadyStopped,
        }
    }

    fn stop(&mut self) -> SourceObservationEffect {
        if self.lifecycle == ObservationLifecycle::Stopped {
            SourceObservationEffect::AlreadyStopped
        } else {
            self.lifecycle = ObservationLifecycle::Stopped;
            self.refresh = SourceRefreshState::Clean;
            self.active_work = None;
            SourceObservationEffect::Stopped
        }
    }

    fn advance_observer_epoch(&mut self) -> Option<NonZeroU64> {
        let next = self.observer_epoch.get().checked_add(1)?;
        self.observer_epoch = NonZeroU64::new(next).expect("a positive value plus one is non-zero");
        Some(self.observer_epoch)
    }

    fn issue_work_token(&mut self) -> Option<SourceWorkToken> {
        let next = self.last_work_generation.checked_add(1)?;
        let work_generation = NonZeroU64::new(next).expect("zero plus one is non-zero");
        self.last_work_generation = next;
        let token = SourceWorkToken {
            observer: self.observer_token(),
            work_generation,
        };
        self.active_work = Some(token);
        Some(token)
    }

    fn fail_closed_at_generation_limit(
        &mut self,
        counter: ExhaustedCoordinationCounter,
    ) -> SourceObservationEffect {
        self.lifecycle = ObservationLifecycle::Stopped;
        self.refresh = SourceRefreshState::Clean;
        self.active_work = None;
        SourceObservationEffect::StoppedAtGenerationLimit { counter }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineDeliveryState {
    Pending,
    InFlight { turn_id: Uuid },
    Acknowledged { session_epoch: NonZeroU64 },
}

/// Immutable operation/context scope shared by semantic events and an Agent timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SemanticContextAuthority {
    operation_id: Uuid,
    context_id: Uuid,
}

impl SemanticContextAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid) -> Self {
        Self {
            operation_id,
            context_id,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }
}

/// One ordered semantic change between two complete projection identities.
#[derive(Debug, PartialEq, Eq)]
pub struct SemanticTimelineEvent {
    authority: SemanticContextAuthority,
    event_id: Uuid,
    base_projection: ProjectionRef,
    target_projection: ProjectionRef,
    payload: serde_json::Value,
    canonical_payload_bytes: Box<[u8]>,
    delivery: TimelineDeliveryState,
}

impl SemanticTimelineEvent {
    pub fn try_new<T: Serialize>(
        authority: SemanticContextAuthority,
        event_id: Uuid,
        base_projection: ProjectionRef,
        target_projection: ProjectionRef,
        payload: &T,
    ) -> Result<Self, SemanticTimelineEventError> {
        if base_projection.revision >= target_projection.revision
            || base_projection.digest == target_projection.digest
        {
            return Err(SemanticTimelineEventError::InvalidProjectionTransition);
        }
        let canonical = CanonicalProjection::from_serializable(payload)
            .map_err(SemanticTimelineEventError::Canonicalization)?;
        let payload = serde_json::from_slice(canonical.bytes())
            .map_err(SemanticTimelineEventError::CanonicalPayloadDecode)?;
        Ok(Self {
            authority,
            event_id,
            base_projection,
            target_projection,
            payload,
            canonical_payload_bytes: canonical.bytes().to_vec().into_boxed_slice(),
            delivery: TimelineDeliveryState::Pending,
        })
    }

    pub fn event_id(&self) -> Uuid {
        self.event_id
    }

    pub fn authority(&self) -> SemanticContextAuthority {
        self.authority
    }

    pub fn base_projection(&self) -> &ProjectionRef {
        &self.base_projection
    }

    pub fn target_projection(&self) -> &ProjectionRef {
        &self.target_projection
    }

    pub fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    pub fn canonical_payload_bytes(&self) -> &[u8] {
        &self.canonical_payload_bytes
    }

    pub fn serialized_payload_bytes(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.canonical_payload_bytes.len())
            .expect("a canonical JSON value is non-empty")
    }

    pub fn delivery(&self) -> TimelineDeliveryState {
        self.delivery
    }
}

#[derive(Debug, Error)]
pub enum SemanticTimelineEventError {
    #[error("semantic events must advance to a different projection identity")]
    InvalidProjectionTransition,
    #[error("semantic event canonicalization failed: {0}")]
    Canonicalization(#[source] CanonicalProjectionError),
    #[error("canonical semantic payload could not be decoded: {0}")]
    CanonicalPayloadDecode(#[source] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryCheckpointReason {
    PauseResume,
    TimelineOverflow,
    TransportUncertain,
    Cancelled,
    MissingResponse,
    AdapterRestart,
    AuthenticationRecovered,
    PolicyReestablished,
    ProjectionChainMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeltaAvailability {
    Available,
    RecoveryCheckpointRequired { reason: RecoveryCheckpointReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineLimits {
    pub max_events: NonZeroUsize,
    pub max_serialized_payload_bytes: NonZeroUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelinePushOutcome {
    Accepted,
    RecoveryCheckpointRequired,
}

/// Immutable ownership scope for one Agent session epoch's semantic timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimelineAuthority {
    operation_id: Uuid,
    context_id: Uuid,
    session_epoch: NonZeroU64,
}

impl TimelineAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid, session_epoch: NonZeroU64) -> Self {
        Self {
            operation_id,
            context_id,
            session_epoch,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }

    pub fn session_epoch(self) -> NonZeroU64 {
        self.session_epoch
    }

    pub fn semantic_context(self) -> SemanticContextAuthority {
        SemanticContextAuthority::new(self.operation_id, self.context_id)
    }
}

/// A finite ordered timeline that never evicts an event not acknowledged by the Agent epoch.
#[derive(Debug)]
pub struct LensSemanticTimeline {
    authority: TimelineAuthority,
    limits: TimelineLimits,
    events: VecDeque<SemanticTimelineEvent>,
    serialized_payload_bytes: usize,
    availability: DeltaAvailability,
    in_flight: Option<AgentTurnKey>,
}

impl LensSemanticTimeline {
    pub fn new(authority: TimelineAuthority, limits: TimelineLimits) -> Self {
        Self {
            authority,
            limits,
            events: VecDeque::new(),
            serialized_payload_bytes: 0,
            availability: DeltaAvailability::Available,
            in_flight: None,
        }
    }

    pub fn availability(&self) -> DeltaAvailability {
        self.availability
    }

    pub fn authority(&self) -> TimelineAuthority {
        self.authority
    }

    pub fn events(&self) -> &VecDeque<SemanticTimelineEvent> {
        &self.events
    }

    pub fn serialized_payload_bytes(&self) -> usize {
        self.serialized_payload_bytes
    }

    pub fn in_flight(&self) -> Option<&AgentTurnKey> {
        self.in_flight.as_ref()
    }

    pub fn push(
        &mut self,
        event: SemanticTimelineEvent,
    ) -> Result<TimelinePushOutcome, TimelineError> {
        if self.availability != DeltaAvailability::Available {
            return Ok(TimelinePushOutcome::RecoveryCheckpointRequired);
        }
        if event.authority.operation_id != self.authority.operation_id {
            return Err(TimelineError::OperationAuthorityMismatch);
        }
        if event.authority.context_id != self.authority.context_id {
            return Err(TimelineError::ContextAuthorityMismatch);
        }
        if let Some(previous) = self.events.back() {
            if previous.target_projection != event.base_projection {
                return Err(TimelineError::ProjectionChainMismatch);
            }
        }

        let event_bytes = event.serialized_payload_bytes().get();
        while self.events.len() >= self.limits.max_events.get()
            || self
                .serialized_payload_bytes
                .checked_add(event_bytes)
                .is_none_or(|total| total > self.limits.max_serialized_payload_bytes.get())
        {
            let may_evict = self.events.front().is_some_and(|event| {
                matches!(event.delivery, TimelineDeliveryState::Acknowledged { .. })
            });
            if !may_evict {
                self.availability = DeltaAvailability::RecoveryCheckpointRequired {
                    reason: RecoveryCheckpointReason::TimelineOverflow,
                };
                return Ok(TimelinePushOutcome::RecoveryCheckpointRequired);
            }
            if let Some(evicted) = self.events.pop_front() {
                self.serialized_payload_bytes = self
                    .serialized_payload_bytes
                    .checked_sub(evicted.serialized_payload_bytes().get())
                    .expect("timeline payload total covers every retained event");
            }
        }

        self.serialized_payload_bytes = self
            .serialized_payload_bytes
            .checked_add(event_bytes)
            .expect("accepted timeline bytes are bounded by the configured maximum");
        self.events.push_back(event);
        Ok(TimelinePushOutcome::Accepted)
    }

    /// Marks the exact ordered pending prefix covered by one delta turn as in-flight.
    pub fn begin_delta_turn(&mut self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        self.validate_turn_authority(key)?;
        if self.availability != DeltaAvailability::Available {
            return Err(TimelineError::RecoveryCheckpointRequired);
        }
        let AgentProjectionCursor::Applied { projection: base } = &key.base_projection else {
            return Err(TimelineError::DeltaRequiresAppliedBase);
        };
        if self.in_flight.is_some() {
            return Err(TimelineError::TurnAlreadyInFlight);
        }

        let first_pending = self
            .events
            .iter()
            .position(|event| event.delivery == TimelineDeliveryState::Pending)
            .ok_or(TimelineError::NoPendingEvents)?;
        if self.events[first_pending].base_projection != *base {
            return Err(TimelineError::ProjectionChainMismatch);
        }

        let target_index = self
            .events
            .iter()
            .enumerate()
            .skip(first_pending)
            .take_while(|(_, event)| event.delivery == TimelineDeliveryState::Pending)
            .find_map(|(index, event)| {
                (event.target_projection == key.target_projection).then_some(index)
            })
            .ok_or(TimelineError::TargetProjectionNotPending)?;

        for event in self.events.range_mut(first_pending..=target_index) {
            event.delivery = TimelineDeliveryState::InFlight {
                turn_id: key.turn_id,
            };
        }
        self.in_flight = Some(key.clone());
        Ok(())
    }

    /// Acknowledges only events previously associated with this exact local turn.
    pub fn acknowledge_turn(&mut self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        self.validate_turn_authority(key)?;
        let Some(in_flight) = self.in_flight.as_ref() else {
            return Err(TimelineError::TurnNotInFlight);
        };
        if in_flight != key {
            return Err(TimelineError::TurnKeyMismatch);
        }
        let matching = self
            .events
            .iter()
            .filter(|event| {
                event.delivery
                    == (TimelineDeliveryState::InFlight {
                        turn_id: key.turn_id,
                    })
            })
            .count();
        if matching == 0 {
            return Err(TimelineError::TurnNotInFlight);
        }
        let final_target_matches = self.events.iter().rev().find(|event| {
            event.delivery
                == (TimelineDeliveryState::InFlight {
                    turn_id: key.turn_id,
                })
        });
        if !final_target_matches
            .is_some_and(|event| event.target_projection == key.target_projection)
        {
            return Err(TimelineError::TargetProjectionMismatch);
        }
        for event in &mut self.events {
            if event.delivery
                == (TimelineDeliveryState::InFlight {
                    turn_id: key.turn_id,
                })
            {
                event.delivery = TimelineDeliveryState::Acknowledged {
                    session_epoch: key.session_epoch,
                };
            }
        }
        self.in_flight = None;
        Ok(())
    }

    /// Latches recovery when the Agent's applied cursor is no longer definitive.
    pub fn require_recovery(&mut self, reason: RecoveryCheckpointReason) {
        if self.availability == DeltaAvailability::Available {
            self.availability = DeltaAvailability::RecoveryCheckpointRequired { reason };
        }
    }

    fn validate_turn_authority(&self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        if key.operation_id != self.authority.operation_id {
            Err(TimelineError::OperationAuthorityMismatch)
        } else if key.context_id != self.authority.context_id {
            Err(TimelineError::ContextAuthorityMismatch)
        } else if key.session_epoch != self.authority.session_epoch {
            Err(TimelineError::EpochAuthorityMismatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error("the turn operation does not own this semantic timeline")]
    OperationAuthorityMismatch,
    #[error("the turn context does not own this semantic timeline")]
    ContextAuthorityMismatch,
    #[error("the turn session epoch does not own this semantic timeline")]
    EpochAuthorityMismatch,
    #[error("semantic timeline projection chain is not contiguous")]
    ProjectionChainMismatch,
    #[error("a delta turn requires an explicitly applied base projection")]
    DeltaRequiresAppliedBase,
    #[error("another timeline turn is already in flight")]
    TurnAlreadyInFlight,
    #[error("the timeline has no pending semantic events")]
    NoPendingEvents,
    #[error("the requested target projection is not an ordered pending target")]
    TargetProjectionNotPending,
    #[error("the requested turn has no in-flight semantic events")]
    TurnNotInFlight,
    #[error("the requested turn key does not equal the active local turn key")]
    TurnKeyMismatch,
    #[error("the in-flight event batch does not end at the turn target projection")]
    TargetProjectionMismatch,
    #[error("the timeline requires a recovery checkpoint")]
    RecoveryCheckpointRequired,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lens::{
            target_id, LensCoordinateSpace, LensInputSource, LensMediaAttachment,
            LensMediaCoverage, LensNodeKind, LensSource, LENS_INPUT_SCHEMA_VERSION,
        },
        model::{SelectedWindow, WindowIdentity, WindowObservableFacts},
    };
    use serde_json::json;

    fn nonzero(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).expect("test value is non-zero")
    }

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
                window_id,
                bundle_id: "example.browser".into(),
                pid: 42,
            },
            facts: WindowObservableFacts {
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
                    bundle_id: "example.browser".into(),
                    window_id,
                },
                document: Some(LensDocumentProjection {
                    nodes: vec![
                        LensContentNode {
                            id: "node-000000".into(),
                            parent_id: None,
                            kind: LensNodeKind::Region,
                            role: Some("AXWindow".into()),
                            subrole: None,
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
                            role: Some("AXImage".into()),
                            subrole: None,
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
                id: attachment_id.clone(),
                target_id,
                uri: uri.clone(),
                scope: LensMediaScope::AxElementRegion,
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

    fn projection(revision: u64, byte: u8) -> ProjectionRef {
        ProjectionRef::new(
            nonzero(revision),
            format!("{byte:02x}")
                .repeat(32)
                .parse()
                .expect("valid digest"),
        )
    }

    fn source_authority(source_registration_id: u128) -> SourceRegistrationAuthority {
        SourceRegistrationAuthority::new(
            Uuid::from_u128(100),
            Uuid::from_u128(101),
            Uuid::from_u128(source_registration_id),
        )
    }

    fn event(event_id: u128, base: ProjectionRef, target: ProjectionRef) -> SemanticTimelineEvent {
        event_for(
            SemanticContextAuthority::new(Uuid::from_u128(10), Uuid::from_u128(11)),
            event_id,
            base,
            target,
        )
    }

    fn event_for(
        authority: SemanticContextAuthority,
        event_id: u128,
        base: ProjectionRef,
        target: ProjectionRef,
    ) -> SemanticTimelineEvent {
        SemanticTimelineEvent::try_new(
            authority,
            Uuid::from_u128(event_id),
            base,
            target,
            &"change",
        )
        .expect("valid event")
    }

    fn combined_event_payload_bytes(events: &[&SemanticTimelineEvent]) -> NonZeroUsize {
        let bytes = events.iter().fold(0usize, |total, event| {
            total
                .checked_add(event.serialized_payload_bytes().get())
                .expect("test event sizes fit usize")
        });
        NonZeroUsize::new(bytes).expect("test events serialize to non-empty JSON")
    }

    fn turn(turn_id: u128, base: ProjectionRef, target: ProjectionRef) -> AgentTurnKey {
        AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            nonzero(1),
            Uuid::from_u128(turn_id),
            AgentProjectionCursor::Applied { projection: base },
            target,
        )
        .expect("valid turn")
    }

    fn timeline(limits: TimelineLimits) -> LensSemanticTimeline {
        LensSemanticTimeline::new(
            TimelineAuthority::new(Uuid::from_u128(10), Uuid::from_u128(11), nonzero(1)),
            limits,
        )
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

    #[test]
    fn source_observation_coalesces_bursts_without_parallel_refreshes() {
        let epoch = nonzero(1);
        let authority = source_authority(102);
        let mut coordinator = SourceObservationCoordinator::new(authority, epoch);
        let observer = coordinator.observer_token();
        let other_observer =
            SourceObservationCoordinator::new(source_authority(103), epoch).observer_token();

        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: other_observer
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::RegistrationMismatch {
                    expected: authority,
                    received: other_observer.authority()
                }
            }
        );

        let first = coordinator.apply(SourceObservationEvent::Notification { observer });
        let SourceObservationEffect::CoalescingStarted { token: first_token } = first else {
            panic!("first invalidation must start coalescing");
        };
        assert_eq!(first_token.observer_epoch(), epoch);
        assert_eq!(first_token.work_generation(), nonzero(1));
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::InvalidationCoalesced { token: first_token }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_token }),
            SourceObservationEffect::RefreshStarted { token: first_token }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::DirtyLatched {
                active: first_token
            }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::DirtyAlreadyLatched {
                active: first_token
            }
        );
        let follow_up =
            coordinator.apply(SourceObservationEvent::RefreshCompleted { token: first_token });
        let SourceObservationEffect::FollowUpCoalescingStarted {
            token: follow_up_token,
        } = follow_up
        else {
            panic!("dirty refresh must start one follow-up coalescing interval");
        };
        assert_ne!(follow_up_token, first_token);
        assert_eq!(follow_up_token.work_generation(), nonzero(2));
        assert_eq!(coordinator.refresh(), SourceRefreshState::Coalescing);
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_token }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::WorkTokenMismatch {
                    expected: Some(follow_up_token),
                    received: first_token
                }
            }
        );
    }

    #[test]
    fn pause_resume_and_stop_reject_obsolete_observer_epochs() {
        let first_epoch = nonzero(1);
        let mut coordinator = SourceObservationCoordinator::new(source_authority(102), first_epoch);
        let first_observer = coordinator.observer_token();
        let SourceObservationEffect::CoalescingStarted { token: first_work } =
            coordinator.apply(SourceObservationEvent::Notification {
                observer: first_observer,
            })
        else {
            panic!("notification must create work");
        };
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_work }),
            SourceObservationEffect::RefreshStarted { token: first_work }
        );
        let paused = coordinator.apply(SourceObservationEvent::Pause);
        assert_eq!(
            paused,
            SourceObservationEffect::Paused {
                observer: coordinator.observer_token()
            }
        );
        assert_eq!(coordinator.observer_epoch(), nonzero(2));
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: first_observer
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: ObservationLifecycle::Paused
                }
            }
        );
        let resumed = coordinator.apply(SourceObservationEvent::Resume);
        let SourceObservationEffect::ResumedWithFullRefresh {
            token: resumed_work,
        } = resumed
        else {
            panic!("resume must start one authoritative full refresh");
        };
        assert_eq!(resumed_work.observer_epoch(), nonzero(3));
        assert_ne!(resumed_work, first_work);
        assert_eq!(coordinator.refresh(), SourceRefreshState::Refreshing);
        assert_eq!(
            coordinator.apply(SourceObservationEvent::RefreshCompleted { token: first_work }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::EpochMismatch {
                    expected: nonzero(3),
                    received: first_work.observer_epoch()
                }
            }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Stop),
            SourceObservationEffect::Stopped
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: resumed_work.observer()
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: ObservationLifecycle::Stopped
                }
            }
        );
    }

    #[test]
    fn observer_epoch_exhaustion_stops_fail_closed() {
        let mut coordinator =
            SourceObservationCoordinator::new(source_authority(102), nonzero(u64::MAX));

        assert_eq!(
            coordinator.apply(SourceObservationEvent::Pause),
            SourceObservationEffect::StoppedAtGenerationLimit {
                counter: ExhaustedCoordinationCounter::ObserverEpoch
            }
        );
        assert_eq!(coordinator.lifecycle(), ObservationLifecycle::Stopped);
        assert_eq!(coordinator.refresh(), SourceRefreshState::Clean);
        assert_eq!(coordinator.observer_epoch(), nonzero(u64::MAX));

        let mut work_exhausted =
            SourceObservationCoordinator::new(source_authority(102), nonzero(1));
        work_exhausted.last_work_generation = u64::MAX;
        let observer = work_exhausted.observer_token();
        assert_eq!(
            work_exhausted.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::StoppedAtGenerationLimit {
                counter: ExhaustedCoordinationCounter::WorkGeneration
            }
        );
        assert_eq!(work_exhausted.lifecycle(), ObservationLifecycle::Stopped);
    }

    #[test]
    fn recovery_checkpoint_reason_has_the_complete_stable_wire_vocabulary() {
        let cases = [
            (RecoveryCheckpointReason::PauseResume, "pause_resume"),
            (
                RecoveryCheckpointReason::TimelineOverflow,
                "timeline_overflow",
            ),
            (
                RecoveryCheckpointReason::TransportUncertain,
                "transport_uncertain",
            ),
            (RecoveryCheckpointReason::Cancelled, "cancelled"),
            (
                RecoveryCheckpointReason::MissingResponse,
                "missing_response",
            ),
            (RecoveryCheckpointReason::AdapterRestart, "adapter_restart"),
            (
                RecoveryCheckpointReason::AuthenticationRecovered,
                "authentication_recovered",
            ),
            (
                RecoveryCheckpointReason::PolicyReestablished,
                "policy_reestablished",
            ),
            (
                RecoveryCheckpointReason::ProjectionChainMismatch,
                "projection_chain_mismatch",
            ),
        ];

        for (reason, expected) in cases {
            assert_eq!(
                serde_json::to_value(reason).expect("serialize recovery reason"),
                serde_json::Value::String(expected.to_owned())
            );
        }
    }

    #[test]
    fn semantic_timeline_event_owns_its_exact_canonical_payload_bytes() {
        let event = event(7, projection(1, 1), projection(2, 2));

        assert_eq!(
            event.serialized_payload_bytes().get(),
            event.canonical_payload_bytes().len()
        );
        assert_eq!(event.canonical_payload_bytes(), br#""change""#);
        assert_eq!(event.payload(), "change");
        assert_eq!(event.delivery(), TimelineDeliveryState::Pending);
    }

    #[test]
    fn recovery_reason_is_a_first_terminal_cause_latch() {
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(1).expect("non-zero"),
            max_serialized_payload_bytes: NonZeroUsize::new(1).expect("non-zero"),
        };
        let mut timeline = timeline(limits);

        timeline.require_recovery(RecoveryCheckpointReason::MissingResponse);
        timeline.require_recovery(RecoveryCheckpointReason::Cancelled);

        assert_eq!(
            timeline.availability(),
            DeltaAvailability::RecoveryCheckpointRequired {
                reason: RecoveryCheckpointReason::MissingResponse
            }
        );
    }

    #[test]
    fn turn_key_is_the_exhaustive_candidate_commit_authority() {
        let base = projection(1, 1);
        let target = projection(2, 2);
        let key = turn(12, base, target.clone());

        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::Authoritative
        );
        let other_turn = turn(13, projection(1, 1), target.clone());
        assert_eq!(
            key.candidate_authority(
                &other_turn,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::TurnKeyMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                Uuid::from_u128(99),
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::OperationMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                Uuid::from_u128(99),
                key.session_epoch,
                &target
            ),
            CandidateAuthority::ContextMismatch
        );
        assert_eq!(
            key.candidate_authority(&key, key.operation_id, key.context_id, nonzero(2), &target),
            CandidateAuthority::EpochMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &projection(3, 3)
            ),
            CandidateAuthority::ProjectionMismatch
        );
        assert_eq!(
            key.applied_cursor_after_definitive_response(),
            AgentProjectionCursor::Applied { projection: target }
        );
    }

    #[test]
    fn unacknowledged_timeline_overflow_latches_recovery_without_data_loss() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let p3 = projection(3, 3);
        let p4 = projection(4, 4);
        let first = event(1, p1, p2.clone());
        let second = event(2, p2, p3.clone());
        let third = event(3, p3, p4);
        let max_serialized_payload_bytes = combined_event_payload_bytes(&[&first, &second]);
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(2).expect("non-zero"),
            max_serialized_payload_bytes,
        };
        let mut timeline = timeline(limits);
        timeline.push(first).expect("first event");
        timeline.push(second).expect("second event");
        let before = timeline
            .events()
            .iter()
            .map(SemanticTimelineEvent::event_id)
            .collect::<Vec<_>>();

        assert_eq!(
            timeline.push(third).expect("finite overflow outcome"),
            TimelinePushOutcome::RecoveryCheckpointRequired
        );
        assert_eq!(
            timeline.availability(),
            DeltaAvailability::RecoveryCheckpointRequired {
                reason: RecoveryCheckpointReason::TimelineOverflow
            }
        );
        assert_eq!(
            timeline
                .events()
                .iter()
                .map(SemanticTimelineEvent::event_id)
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(
            timeline.serialized_payload_bytes(),
            max_serialized_payload_bytes.get()
        );
    }

    #[test]
    fn only_acknowledged_prefix_is_evictable() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let p3 = projection(3, 3);
        let p4 = projection(4, 4);
        let first = event(1, p1.clone(), p2.clone());
        let second = event(2, p2.clone(), p3.clone());
        let third = event(3, p3, p4);
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(2).expect("non-zero"),
            max_serialized_payload_bytes: combined_event_payload_bytes(&[&first, &second]),
        };
        let mut timeline = timeline(limits);
        timeline.push(first).expect("first event");
        timeline.push(second).expect("second event");
        let key = turn(20, p1, p2);
        timeline.begin_delta_turn(&key).expect("start first turn");
        timeline
            .acknowledge_turn(&key)
            .expect("acknowledge first turn");

        assert_eq!(
            timeline
                .push(third)
                .expect("acknowledged prefix can be evicted"),
            TimelinePushOutcome::Accepted
        );
        assert_eq!(timeline.events().len(), 2);
        assert_eq!(
            timeline.events().front().expect("event").event_id(),
            Uuid::from_u128(2)
        );
        assert_eq!(timeline.availability(), DeltaAvailability::Available);
    }

    #[test]
    fn timeline_acknowledgement_requires_the_exact_local_turn() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let event = event(1, p1.clone(), p2.clone());
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(4).expect("non-zero"),
            max_serialized_payload_bytes: combined_event_payload_bytes(&[&event]),
        };
        let mut timeline = timeline(limits);
        let foreign_operation_event = event_for(
            SemanticContextAuthority::new(Uuid::from_u128(99), Uuid::from_u128(11)),
            2,
            p1.clone(),
            p2.clone(),
        );
        assert_eq!(
            timeline.push(foreign_operation_event),
            Err(TimelineError::OperationAuthorityMismatch)
        );
        let foreign_context_event = event_for(
            SemanticContextAuthority::new(Uuid::from_u128(10), Uuid::from_u128(99)),
            3,
            p1.clone(),
            p2.clone(),
        );
        assert_eq!(
            timeline.push(foreign_context_event),
            Err(TimelineError::ContextAuthorityMismatch)
        );
        timeline.push(event).expect("event");
        let key = turn(20, p1.clone(), p2.clone());
        let wrong_operation = AgentTurnKey::try_new(
            Uuid::from_u128(99),
            Uuid::from_u128(11),
            nonzero(1),
            Uuid::from_u128(19),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_operation),
            Err(TimelineError::OperationAuthorityMismatch)
        );
        let wrong_context = AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(99),
            nonzero(1),
            Uuid::from_u128(18),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign context turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_context),
            Err(TimelineError::ContextAuthorityMismatch)
        );
        let wrong_epoch = AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            nonzero(2),
            Uuid::from_u128(17),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign epoch turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_epoch),
            Err(TimelineError::EpochAuthorityMismatch)
        );
        assert_eq!(
            timeline.events().front().expect("event").delivery(),
            TimelineDeliveryState::Pending
        );
        timeline.begin_delta_turn(&key).expect("start turn");
        let wrong_key = turn(21, p1, p2);

        assert_eq!(
            timeline.acknowledge_turn(&wrong_key),
            Err(TimelineError::TurnKeyMismatch)
        );
        assert_eq!(
            timeline.events().front().expect("event").delivery(),
            TimelineDeliveryState::InFlight {
                turn_id: key.turn_id
            }
        );
    }
}
