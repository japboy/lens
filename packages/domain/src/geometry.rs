//! Independent canonical geometry values; no platform-port dependency.
use std::num::NonZeroU64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct TargetReceipt(Uuid);

impl TryFrom<Uuid> for TargetReceipt {
    type Error = &'static str;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        if value.is_nil() {
            Err("target receipt must not be nil")
        } else {
            Ok(Self(value))
        }
    }
}
impl From<TargetReceipt> for Uuid {
    fn from(value: TargetReceipt) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAuthority {
    pub operation_id: Uuid,
    pub receipt: TargetReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReadSequence(NonZeroU64);

impl TryFrom<String> for ReadSequence {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parsed = value
            .parse::<NonZeroU64>()
            .map_err(|_| "read sequence must be a nonzero canonical decimal u64 string")?;
        if parsed.to_string() != value {
            return Err("read sequence must be a nonzero canonical decimal u64 string");
        }
        Ok(Self(parsed))
    }
}
impl From<ReadSequence> for String {
    fn from(value: ReadSequence) -> Self {
        value.0.to_string()
    }
}
impl ReadSequence {
    pub const FIRST: Self = Self(NonZeroU64::MIN);
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetReadKey {
    pub target: TargetAuthority,
    pub sequence: ReadSequence,
}
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Selection facts have units, but do not claim membership in an acquisition read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFrame {
    MacosDesktopPoints,
    WindowsDesktopPhysicalPixels,
}

impl DesktopFrame {
    fn coordinate_frame(self) -> CoordinateFrame {
        match self {
            Self::MacosDesktopPoints => CoordinateFrame::MacosDesktopPoints,
            Self::WindowsDesktopPhysicalPixels => CoordinateFrame::WindowsDesktopPhysicalPixels,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopRect {
    pub frame: DesktopFrame,
    pub rect: Rect,
}

impl DesktopRect {
    pub fn validate(&self) -> Result<(), GeometryError> {
        self.rect.validate()
    }
}

/// Node coordinates distinguish acquisition authority from explicit diagnostic facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeBounds {
    Registered { geometry: TaggedRect },
    Legacy { geometry: DesktopRect },
}

impl NodeBounds {
    pub fn validate(&self) -> Result<(), GeometryError> {
        match self {
            Self::Registered { geometry } => geometry.validate(),
            Self::Legacy { geometry } => geometry.validate(),
        }
    }

    /// None admits only explicit legacy facts, never a registered fallback.
    pub fn validate_for(
        &self,
        descriptor: Option<&ReadGeometryDescriptor>,
    ) -> Result<(), GeometryError> {
        self.validate()?;
        match (self, descriptor) {
            (Self::Legacy { .. }, None) => Ok(()),
            (Self::Registered { geometry }, Some(descriptor)) => {
                descriptor.validate()?;
                if geometry.read != descriptor.read {
                    return Err(GeometryError::AuthorityMismatch);
                }
                if geometry.frame != descriptor.window.frame {
                    return Err(GeometryError::FrameMismatch);
                }
                Ok(())
            }
            _ => Err(GeometryError::AuthorityMismatch),
        }
    }
}

/// Native acquisition observations, never synthesized by attaching a read to picker facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadGeometryDescriptor {
    pub read: TargetReadKey,
    pub window: TaggedRect,
    pub desktop_to_target: AxisAlignedTransform,
}

impl ReadGeometryDescriptor {
    pub fn validate(&self) -> Result<(), GeometryError> {
        validate_read(self.read)?;
        self.window.validate()?;
        self.desktop_to_target.validate()?;
        if self.window.read != self.read || self.desktop_to_target.read != self.read {
            return Err(GeometryError::AuthorityMismatch);
        }
        let desktop = match self.window.frame {
            CoordinateFrame::MacosDesktopPoints => DesktopFrame::MacosDesktopPoints,
            CoordinateFrame::WindowsDesktopPhysicalPixels => {
                DesktopFrame::WindowsDesktopPhysicalPixels
            }
            _ => return Err(GeometryError::FrameMismatch),
        };
        if self.desktop_to_target.source != desktop.coordinate_frame()
            || self.desktop_to_target.destination
                != (CoordinateFrame::TargetLogical {
                    target: self.read.target,
                })
        {
            return Err(GeometryError::FrameMismatch);
        }
        // A target-relative window begins at its own origin. Scales remain supplied
        // native facts, not inferred from the desktop unit or a presumed DPI.
        if self.desktop_to_target.translate_x
            != -self.desktop_to_target.scale_x * self.window.rect.x
            || self.desktop_to_target.translate_y
                != -self.desktop_to_target.scale_y * self.window.rect.y
        {
            return Err(GeometryError::InvalidTransform);
        }
        self.desktop_to_target.apply(&self.window)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureGeometryExpectation {
    ObserveCurrent,
    MatchExtraction {
        descriptor: Box<ReadGeometryDescriptor>,
    },
}

impl CaptureGeometryExpectation {
    pub fn validate_for(&self, read: TargetReadKey) -> Result<(), GeometryError> {
        validate_read(read)?;
        match self {
            Self::ObserveCurrent => Ok(()),
            Self::MatchExtraction { descriptor } => {
                descriptor.validate()?;
                if descriptor.read != read {
                    return Err(GeometryError::AuthorityMismatch);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureKey {
    pub read: TargetReadKey,
    pub capture_id: Uuid,
}

/// All frames have increasing x rightward and y downward; adapters normalize orientation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoordinateFrame {
    MacosDesktopPoints,
    WindowsDesktopPhysicalPixels,
    TargetLogical {
        target: TargetAuthority,
    },
    OriginalCapturePixels {
        capture: CaptureKey,
    },
    CroppedAttachmentPixels {
        capture: CaptureKey,
        attachment_id: String,
    },
    EncodedAttachmentPixels {
        capture: CaptureKey,
        attachment_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaggedRect {
    pub read: TargetReadKey,
    pub frame: CoordinateFrame,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisAlignedTransform {
    pub read: TargetReadKey,
    pub source: CoordinateFrame,
    pub destination: CoordinateFrame,
    pub scale_x: f64,
    pub scale_y: f64,
    pub translate_x: f64,
    pub translate_y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Intersection {
    Empty,
    Overlap(TaggedRect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GeometryError {
    #[error("geometry identity is missing or invalid")]
    InvalidIdentity,
    #[error("geometry belongs to a different target or read")]
    AuthorityMismatch,
    #[error("coordinate frames do not match")]
    FrameMismatch,
    #[error("rectangle edges must be finite with positive dimensions")]
    InvalidRect,
    #[error("transform requires positive finite scales and finite offsets")]
    InvalidTransform,
    #[error("pixel dimensions or crop containment are invalid")]
    InvalidPixels,
    #[error("pixel edge transform does not match capture metadata")]
    PixelTransformMismatch,
}

fn validate_read(read: TargetReadKey) -> Result<(), GeometryError> {
    if read.target.operation_id.is_nil() {
        Err(GeometryError::InvalidIdentity)
    } else {
        Ok(())
    }
}

impl CoordinateFrame {
    fn capture(&self) -> Option<CaptureKey> {
        match self {
            Self::OriginalCapturePixels { capture }
            | Self::CroppedAttachmentPixels { capture, .. }
            | Self::EncodedAttachmentPixels { capture, .. } => Some(*capture),
            _ => None,
        }
    }

    fn attachment(&self) -> Option<&str> {
        match self {
            Self::CroppedAttachmentPixels { attachment_id, .. }
            | Self::EncodedAttachmentPixels { attachment_id, .. } => Some(attachment_id),
            _ => None,
        }
    }
    pub fn validate_for(&self, read: TargetReadKey) -> Result<(), GeometryError> {
        validate_read(read)?;
        let capture = match self {
            Self::MacosDesktopPoints | Self::WindowsDesktopPhysicalPixels => return Ok(()),
            Self::TargetLogical { target } => {
                return if *target == read.target {
                    Ok(())
                } else {
                    Err(GeometryError::AuthorityMismatch)
                }
            }
            Self::OriginalCapturePixels { capture } => capture,
            Self::CroppedAttachmentPixels {
                capture,
                attachment_id,
            }
            | Self::EncodedAttachmentPixels {
                capture,
                attachment_id,
            } => {
                if attachment_id.is_empty() {
                    return Err(GeometryError::InvalidIdentity);
                }
                capture
            }
        };
        if capture.capture_id.is_nil() {
            return Err(GeometryError::InvalidIdentity);
        }
        if capture.read != read {
            return Err(GeometryError::AuthorityMismatch);
        }
        Ok(())
    }
}

impl Rect {
    pub fn validate(self) -> Result<(), GeometryError> {
        if [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width > self.x
            && self.y + self.height > self.y
        {
            Ok(())
        } else {
            Err(GeometryError::InvalidRect)
        }
    }
}

impl TaggedRect {
    pub fn validate(&self) -> Result<(), GeometryError> {
        self.frame.validate_for(self.read)?;
        self.rect.validate()
    }

    pub fn intersect(&self, other: &Self) -> Result<Intersection, GeometryError> {
        self.validate()?;
        other.validate()?;
        if self.read != other.read {
            return Err(GeometryError::AuthorityMismatch);
        }
        if self.frame != other.frame {
            return Err(GeometryError::FrameMismatch);
        }
        let x = self.rect.x.max(other.rect.x);
        let y = self.rect.y.max(other.rect.y);
        let right = (self.rect.x + self.rect.width).min(other.rect.x + other.rect.width);
        let bottom = (self.rect.y + self.rect.height).min(other.rect.y + other.rect.height);
        if right <= x || bottom <= y {
            return Ok(Intersection::Empty);
        }
        let rect = Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        };
        rect.validate()?;
        Ok(Intersection::Overlap(Self {
            rect,
            ..self.clone()
        }))
    }
}

impl AxisAlignedTransform {
    pub fn validate(&self) -> Result<(), GeometryError> {
        self.source.validate_for(self.read)?;
        self.destination.validate_for(self.read)?;
        if matches!((self.source.capture(), self.destination.capture()), (Some(a), Some(b)) if a != b)
            || matches!((self.source.attachment(), self.destination.attachment()), (Some(a), Some(b)) if a != b)
        {
            return Err(GeometryError::FrameMismatch);
        }
        if [
            self.scale_x,
            self.scale_y,
            self.translate_x,
            self.translate_y,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.scale_x > 0.0
            && self.scale_y > 0.0
        {
            Ok(())
        } else {
            Err(GeometryError::InvalidTransform)
        }
    }

    pub fn apply(&self, input: &TaggedRect) -> Result<TaggedRect, GeometryError> {
        self.validate()?;
        input.validate()?;
        if input.read != self.read {
            return Err(GeometryError::AuthorityMismatch);
        }
        if input.frame != self.source {
            return Err(GeometryError::FrameMismatch);
        }
        let x = self.scale_x * input.rect.x + self.translate_x;
        let y = self.scale_y * input.rect.y + self.translate_y;
        let right = self.scale_x * (input.rect.x + input.rect.width) + self.translate_x;
        let bottom = self.scale_y * (input.rect.y + input.rect.height) + self.translate_y;
        let output = TaggedRect {
            read: self.read,
            frame: self.destination.clone(),
            rect: Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            },
        };
        output.validate()?;
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelExtent {
    pub width: u32,
    pub height: u32,
}

impl PixelExtent {
    pub fn validate(self) -> Result<(), GeometryError> {
        if self.width == 0 || self.height == 0 {
            Err(GeometryError::InvalidPixels)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelCrop {
    pub x: u32,
    pub y: u32,
    pub extent: PixelExtent,
}

impl PixelCrop {
    pub fn validate_within(self, original: PixelExtent) -> Result<(), GeometryError> {
        self.extent.validate()?;
        original.validate()?;
        if self
            .x
            .checked_add(self.extent.width)
            .is_some_and(|right| right <= original.width)
            && self
                .y
                .checked_add(self.extent.height)
                .is_some_and(|bottom| bottom <= original.height)
        {
            Ok(())
        } else {
            Err(GeometryError::InvalidPixels)
        }
    }
}

/// Pixel-edge relationships, not a claim about resampling kernels or requested coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedPixelGeometry {
    pub original_extent: PixelExtent,
    pub crop: PixelCrop,
    pub encoded_extent: PixelExtent,
}

impl CapturedPixelGeometry {
    pub fn validate(self) -> Result<(), GeometryError> {
        self.crop.validate_within(self.original_extent)?;
        self.encoded_extent.validate()
    }
}

/// Pixel-edge relationships, not a claim about resampling kernels or requested coverage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentGeometry {
    pub capture: CaptureKey,
    pub attachment_id: String,
    pub original_extent: PixelExtent,
    pub crop: PixelCrop,
    pub encoded_extent: PixelExtent,
    pub original_to_crop: AxisAlignedTransform,
    pub crop_to_encoded: AxisAlignedTransform,
}

impl AttachmentGeometry {
    pub fn validate(&self) -> Result<(), GeometryError> {
        self.crop.validate_within(self.original_extent)?;
        self.encoded_extent.validate()?;
        let original = CoordinateFrame::OriginalCapturePixels {
            capture: self.capture,
        };
        let cropped = CoordinateFrame::CroppedAttachmentPixels {
            capture: self.capture,
            attachment_id: self.attachment_id.clone(),
        };
        let encoded = CoordinateFrame::EncodedAttachmentPixels {
            capture: self.capture,
            attachment_id: self.attachment_id.clone(),
        };
        let expected_crop = AxisAlignedTransform {
            read: self.capture.read,
            source: original,
            destination: cropped.clone(),
            scale_x: 1.0,
            scale_y: 1.0,
            translate_x: -f64::from(self.crop.x),
            translate_y: -f64::from(self.crop.y),
        };
        let expected_encode = AxisAlignedTransform {
            read: self.capture.read,
            source: cropped,
            destination: encoded,
            scale_x: f64::from(self.encoded_extent.width) / f64::from(self.crop.extent.width),
            scale_y: f64::from(self.encoded_extent.height) / f64::from(self.crop.extent.height),
            translate_x: 0.0,
            translate_y: 0.0,
        };
        self.original_to_crop.validate()?;
        self.crop_to_encoded.validate()?;
        if self.original_to_crop != expected_crop || self.crop_to_encoded != expected_encode {
            return Err(GeometryError::PixelTransformMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_bounds_require_explicit_variant_and_reject_old_raw_coordinates() {
        for json in [
            r#"{"x":0,"y":0,"width":1,"height":1}"#,
            r#"{"kind":"unknown","geometry":{}}"#,
            r#"{"kind":"registered"}"#,
        ] {
            assert!(serde_json::from_str::<NodeBounds>(json).is_err());
        }
        let legacy = NodeBounds::Legacy {
            geometry: DesktopRect {
                frame: DesktopFrame::MacosDesktopPoints,
                rect: rect().rect,
            },
        };
        let mut value = serde_json::to_value(&legacy).unwrap();
        value["read"] = serde_json::to_value(read()).unwrap();
        assert!(serde_json::from_value::<NodeBounds>(value).is_err());
        let value = serde_json::to_value(&legacy).unwrap();
        assert_eq!(serde_json::from_value::<NodeBounds>(value).unwrap(), legacy);
        assert_eq!(legacy.validate_for(None), Ok(()));
        assert_eq!(
            legacy.validate_for(Some(&descriptor())),
            Err(GeometryError::AuthorityMismatch)
        );
    }

    #[test]
    fn registered_node_bounds_match_descriptor_and_never_downgrade_to_legacy() {
        let registered = NodeBounds::Registered { geometry: rect() };
        assert_eq!(registered.validate_for(Some(&descriptor())), Ok(()));
        assert_eq!(
            registered.validate_for(None),
            Err(GeometryError::AuthorityMismatch)
        );
        let mut changed = descriptor();
        changed.read.sequence = changed.read.sequence.checked_next().unwrap();
        changed.window.read = changed.read;
        changed.desktop_to_target.read = changed.read;
        assert_eq!(
            registered.validate_for(Some(&changed)),
            Err(GeometryError::AuthorityMismatch)
        );
        let wrong_frame = NodeBounds::Registered {
            geometry: TaggedRect {
                frame: CoordinateFrame::WindowsDesktopPhysicalPixels,
                ..rect()
            },
        };
        assert_eq!(
            wrong_frame.validate_for(Some(&descriptor())),
            Err(GeometryError::FrameMismatch)
        );
        let invalid = NodeBounds::Registered {
            geometry: TaggedRect {
                rect: Rect {
                    width: f64::NAN,
                    ..rect().rect
                },
                ..rect()
            },
        };
        assert_eq!(invalid.validate(), Err(GeometryError::InvalidRect));
        let mut value = serde_json::to_value(registered).unwrap();
        value["geometry"]["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<NodeBounds>(value).is_err());
    }

    #[test]
    fn canonical_decimal_reads_preserve_all_bits_and_never_wrap() {
        for invalid in [
            "1",
            "0",
            "null",
            "\"0\"",
            "\"01\"",
            "\"+1\"",
            "\" 1\"",
            "\"18446744073709551616\"",
        ] {
            assert!(serde_json::from_str::<ReadSequence>(invalid).is_err());
        }
        assert_eq!(
            serde_json::to_string(&ReadSequence::FIRST).unwrap(),
            "\"1\""
        );
        assert_eq!(
            serde_json::to_string(&ReadSequence::FIRST.checked_next().unwrap()).unwrap(),
            "\"2\""
        );
        let maximum = ReadSequence::try_from(u64::MAX.to_string()).unwrap();
        let encoded = serde_json::to_string(&maximum).unwrap();
        assert_eq!(encoded, "\"18446744073709551615\"");
        assert_eq!(
            serde_json::from_str::<ReadSequence>(&encoded).unwrap(),
            maximum
        );
        assert_eq!(maximum.checked_next(), None);
    }

    #[test]
    fn canonical_geometry_rejects_nil_receipts_and_unknown_authority_fields() {
        assert!(TargetReceipt::try_from(Uuid::nil()).is_err());
        assert!(
            serde_json::from_str::<TargetReceipt>("\"00000000-0000-0000-0000-000000000000\"")
                .is_err()
        );
        let mut value = serde_json::to_value(read()).unwrap();
        value["target"]["window_id"] = serde_json::json!(1);
        assert!(serde_json::from_value::<TargetReadKey>(value).is_err());
        let mut value = serde_json::to_value(read()).unwrap();
        value.as_object_mut().unwrap().remove("sequence");
        assert!(serde_json::from_value::<TargetReadKey>(value).is_err());
    }

    fn read() -> TargetReadKey {
        TargetReadKey {
            target: TargetAuthority {
                operation_id: Uuid::from_u128(1),
                receipt: Uuid::from_u128(2).try_into().unwrap(),
            },
            sequence: ReadSequence::FIRST,
        }
    }
    fn rect() -> TaggedRect {
        TaggedRect {
            read: read(),
            frame: CoordinateFrame::MacosDesktopPoints,
            rect: Rect {
                x: -100.25,
                y: -20.5,
                width: 10.5,
                height: 4.25,
            },
        }
    }
    fn transform() -> AxisAlignedTransform {
        AxisAlignedTransform {
            read: read(),
            source: CoordinateFrame::MacosDesktopPoints,
            destination: CoordinateFrame::TargetLogical {
                target: read().target,
            },
            scale_x: 2.0,
            scale_y: 3.0,
            translate_x: 200.5,
            translate_y: 61.5,
        }
    }
    fn descriptor() -> ReadGeometryDescriptor {
        ReadGeometryDescriptor {
            read: read(),
            window: rect(),
            desktop_to_target: transform(),
        }
    }

    #[test]
    fn selection_geometry_has_units_but_no_acquisition_read() {
        let selection = DesktopRect {
            frame: DesktopFrame::MacosDesktopPoints,
            rect: rect().rect,
        };
        assert_eq!(selection.validate(), Ok(()));
        let mut value = serde_json::to_value(selection).unwrap();
        assert!(value.get("read").is_none());
        value["read"] = serde_json::to_value(read()).unwrap();
        assert!(serde_json::from_value::<DesktopRect>(value).is_err());
        assert!(serde_json::from_str::<DesktopRect>(
            r#"{"rect":{"x":0,"y":0,"width":1,"height":1}}"#
        )
        .is_err());
    }

    #[test]
    fn acquired_descriptor_requires_coherent_read_window_and_transform() {
        assert_eq!(descriptor().validate(), Ok(()));
        let mut bad = descriptor();
        bad.read.sequence = bad.read.sequence.checked_next().unwrap();
        assert_eq!(bad.validate(), Err(GeometryError::AuthorityMismatch));
        let mut bad = descriptor();
        bad.desktop_to_target.source = CoordinateFrame::WindowsDesktopPhysicalPixels;
        assert_eq!(bad.validate(), Err(GeometryError::FrameMismatch));
        let mut bad = descriptor();
        bad.desktop_to_target.destination = CoordinateFrame::WindowsDesktopPhysicalPixels;
        assert_eq!(bad.validate(), Err(GeometryError::FrameMismatch));
        let mut bad = descriptor();
        bad.window.frame = CoordinateFrame::TargetLogical {
            target: read().target,
        };
        assert_eq!(bad.validate(), Err(GeometryError::FrameMismatch));
        let mut bad = descriptor();
        bad.window.rect.x += 1.0;
        assert_eq!(bad.validate(), Err(GeometryError::InvalidTransform));
        let mut bad = descriptor();
        bad.desktop_to_target.scale_x = f64::MAX;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn capture_expectation_never_defaults_or_rebinds_an_extraction() {
        let expected = CaptureGeometryExpectation::MatchExtraction {
            descriptor: Box::new(descriptor()),
        };
        assert_eq!(expected.validate_for(read()), Ok(()));
        let stale = TargetReadKey {
            sequence: read().sequence.checked_next().unwrap(),
            ..read()
        };
        assert_eq!(
            expected.validate_for(stale),
            Err(GeometryError::AuthorityMismatch)
        );
        assert_eq!(
            CaptureGeometryExpectation::ObserveCurrent.validate_for(read()),
            Ok(())
        );
        let mut invalid = read();
        invalid.target.operation_id = Uuid::nil();
        assert_eq!(
            CaptureGeometryExpectation::ObserveCurrent.validate_for(invalid),
            Err(GeometryError::InvalidIdentity)
        );
        for json in [
            r#"{}"#,
            r#"{"kind":"match_extraction"}"#,
            r#"{"kind":"unknown"}"#,
        ] {
            assert!(serde_json::from_str::<CaptureGeometryExpectation>(json).is_err());
        }
        let mut value = serde_json::to_value(descriptor()).unwrap();
        value.as_object_mut().unwrap().remove("desktop_to_target");
        assert!(serde_json::from_value::<ReadGeometryDescriptor>(value).is_err());
    }
    #[test]
    fn negative_fractional_edges_and_unequal_scales_are_explicit() {
        let result = transform().apply(&rect()).unwrap();
        assert_eq!(
            result.rect,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 21.0,
                height: 12.75
            }
        );
        assert_eq!(result.frame, transform().destination);
    }
    #[test]
    fn invalid_geometry_is_not_an_empty_intersection() {
        let a = rect();
        let mut b = a.clone();
        b.rect.x += b.rect.width;
        assert_eq!(a.intersect(&b), Ok(Intersection::Empty));
        b.rect.x -= 1.0;
        assert!(matches!(a.intersect(&b), Ok(Intersection::Overlap(_))));
        b.frame = CoordinateFrame::WindowsDesktopPhysicalPixels;
        assert_eq!(a.intersect(&b), Err(GeometryError::FrameMismatch));
        b = a.clone();
        b.read.sequence = b.read.sequence.checked_next().unwrap();
        assert_eq!(a.intersect(&b), Err(GeometryError::AuthorityMismatch));
        b = a.clone();
        b.rect.width = f64::NAN;
        assert_eq!(a.intersect(&b), Err(GeometryError::InvalidRect));
    }
    #[test]
    fn transforms_reject_wrong_authority_stale_read_and_overflow() {
        let mut t = transform();
        t.destination = CoordinateFrame::TargetLogical {
            target: TargetAuthority {
                operation_id: Uuid::from_u128(3),
                ..read().target
            },
        };
        assert_eq!(t.validate(), Err(GeometryError::AuthorityMismatch));
        t = transform();
        t.read.sequence = t.read.sequence.checked_next().unwrap();
        assert_eq!(t.apply(&rect()), Err(GeometryError::AuthorityMismatch));
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            t = transform();
            t.scale_x = scale;
            assert_eq!(t.validate(), Err(GeometryError::InvalidTransform));
        }
        t = transform();
        t.scale_x = f64::MAX;
        assert_eq!(t.apply(&rect()), Err(GeometryError::InvalidRect));
        let bad = Rect {
            x: f64::MAX,
            y: 0.0,
            width: f64::MAX,
            height: 1.0,
        };
        assert_eq!(bad.validate(), Err(GeometryError::InvalidRect));
        assert_eq!(
            Rect {
                x: 1e30,
                y: 0.0,
                width: 1.0,
                height: 1.0
            }
            .validate(),
            Err(GeometryError::InvalidRect)
        );
    }
    fn attachment() -> AttachmentGeometry {
        let capture = CaptureKey {
            read: read(),
            capture_id: Uuid::from_u128(4),
        };
        let original = CoordinateFrame::OriginalCapturePixels { capture };
        let cropped = CoordinateFrame::CroppedAttachmentPixels {
            capture,
            attachment_id: "a".into(),
        };
        let encoded = CoordinateFrame::EncodedAttachmentPixels {
            capture,
            attachment_id: "a".into(),
        };
        AttachmentGeometry {
            capture,
            attachment_id: "a".into(),
            original_extent: PixelExtent {
                width: 100,
                height: 80,
            },
            crop: PixelCrop {
                x: 10,
                y: 20,
                extent: PixelExtent {
                    width: 40,
                    height: 30,
                },
            },
            encoded_extent: PixelExtent {
                width: 20,
                height: 10,
            },
            original_to_crop: AxisAlignedTransform {
                read: read(),
                source: original,
                destination: cropped.clone(),
                scale_x: 1.0,
                scale_y: 1.0,
                translate_x: -10.0,
                translate_y: -20.0,
            },
            crop_to_encoded: AxisAlignedTransform {
                read: read(),
                source: cropped,
                destination: encoded,
                scale_x: 0.5,
                scale_y: 1.0 / 3.0,
                translate_x: 0.0,
                translate_y: 0.0,
            },
        }
    }
    #[test]
    fn capture_crop_and_resize_metadata_must_agree_exactly() {
        let a = attachment();
        assert_eq!(a.validate(), Ok(()));
        let source = TaggedRect {
            read: read(),
            frame: a.original_to_crop.source.clone(),
            rect: Rect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 30.0,
            },
        };
        let final_rect = a
            .crop_to_encoded
            .apply(&a.original_to_crop.apply(&source).unwrap())
            .unwrap();
        assert_eq!(
            final_rect.rect,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0
            }
        );
        let mut bad = a.clone();
        bad.crop.x = u32::MAX;
        assert_eq!(bad.validate(), Err(GeometryError::InvalidPixels));
        bad = a.clone();
        bad.encoded_extent.width = 0;
        assert_eq!(bad.validate(), Err(GeometryError::InvalidPixels));
        bad = a.clone();
        bad.crop_to_encoded.scale_x = 1.0;
        assert_eq!(bad.validate(), Err(GeometryError::PixelTransformMismatch));
        bad = a.clone();
        bad.attachment_id = "b".into();
        assert_eq!(bad.validate(), Err(GeometryError::PixelTransformMismatch));
        bad = a;
        bad.capture.read.sequence = bad.capture.read.sequence.checked_next().unwrap();
        assert_eq!(bad.validate(), Err(GeometryError::PixelTransformMismatch));
    }

    #[test]
    fn generic_transforms_reject_different_captures_and_attachments() {
        let mut transform = attachment().crop_to_encoded;
        if let CoordinateFrame::EncodedAttachmentPixels { capture, .. } = &mut transform.destination
        {
            capture.capture_id = Uuid::from_u128(9);
        }
        assert_eq!(transform.validate(), Err(GeometryError::FrameMismatch));
        let mut transform = attachment().crop_to_encoded;
        if let CoordinateFrame::EncodedAttachmentPixels { attachment_id, .. } =
            &mut transform.destination
        {
            *attachment_id = "another-attachment".into();
        }
        assert_eq!(transform.validate(), Err(GeometryError::FrameMismatch));
    }
    #[test]
    fn frames_and_transform_fields_have_no_implicit_defaults() {
        assert!(serde_json::from_str::<TaggedRect>(
            r#"{"rect":{"x":0,"y":0,"width":1,"height":1}}"#
        )
        .is_err());
        let mut value = serde_json::to_value(transform()).unwrap();
        value.as_object_mut().unwrap().remove("scale_x");
        assert!(serde_json::from_value::<AxisAlignedTransform>(value).is_err());
        let mut value = serde_json::to_value(rect()).unwrap();
        value["frame"]["kind"] = serde_json::json!("screen");
        assert!(serde_json::from_value::<TaggedRect>(value).is_err());
        let mut invalid = rect();
        invalid.read.target.operation_id = Uuid::nil();
        assert_eq!(invalid.validate(), Err(GeometryError::InvalidIdentity));
    }
}
