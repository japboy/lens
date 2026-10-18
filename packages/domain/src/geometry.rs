//! Geometry belongs to the acquired sample, not a later window query.
//!
//! Desktop coordinates are top-left `desktop_logical` or
//! `desktop_physical_pixels`; sample-local coordinates are `frame_pixels`.
//! Cropped/resized PNG coordinates are `encoded_pixels`. These observations
//! correlate a tree and image; they do not make acquisition atomic.
//! All axes increase rightward/downward. Desktop coordinates retain the native
//! global desktop origin (negative coordinates are allowed); they are not
//! rebased to a virtual desktop's minimum. Frame and encoded origins are (0, 0)
//! at their top-left edges. A platform must establish that mapping explicitly;
//! these unit names do not prove any native API or multi-display conversion.

use crate::model::Bounds;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An opaque sample correlation ID, never target authority or geometry proof.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct FrameId(Uuid);

impl TryFrom<Uuid> for FrameId {
    type Error = &'static str;
    fn try_from(id: Uuid) -> Result<Self, Self::Error> {
        if id.is_nil() {
            Err("frame ID must not be nil")
        } else {
            Ok(Self(id))
        }
    }
}

impl From<FrameId> for Uuid {
    fn from(id: FrameId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSpace {
    DesktopLogical,
    DesktopPhysicalPixels,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PixelExtent {
    pub width: u32,
    pub height: u32,
}

impl PixelExtent {
    pub fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Integer source-frame crop. The encoded image can have different dimensions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn within(self, extent: PixelExtent) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|x| x <= extent.width)
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|y| y <= extent.height)
    }

    /// Intersect first, floor near edges, ceil far edges, then bound to pixels.
    pub fn covering(bounds: Bounds, extent: PixelExtent) -> Option<Self> {
        if !valid_bounds(bounds) || !extent.is_valid() {
            return None;
        }
        let x = bounds.x.max(0.0).floor();
        let y = bounds.y.max(0.0).floor();
        let right = (bounds.x + bounds.width)
            .min(f64::from(extent.width))
            .ceil();
        let bottom = (bounds.y + bounds.height)
            .min(f64::from(extent.height))
            .ceil();
        if right <= x || bottom <= y {
            return None;
        }
        let rect = Self {
            x: x as u32,
            y: y as u32,
            width: (right - x) as u32,
            height: (bottom - y) as u32,
        };
        rect.within(extent).then_some(rect)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GeometryUnavailableReason {
    MissingMetadata,
    InvalidMetadata,
    UnverifiedConfiguration,
}

/// Only positive axis scaling and offsets are supported, not an affine graph.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesktopMapping {
    Known {
        space: DesktopSpace,
        desktop_bounds: Bounds,
        scale_x: f64,
        scale_y: f64,
    },
    Unavailable {
        reason: GeometryUnavailableReason,
    },
}

impl DesktopMapping {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Known {
                desktop_bounds,
                scale_x,
                scale_y,
                ..
            } => {
                valid_bounds(*desktop_bounds)
                    && scale_x.is_finite()
                    && *scale_x > 0.0
                    && scale_y.is_finite()
                    && *scale_y > 0.0
            }
            Self::Unavailable { .. } => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AcquiredFrame {
    pub id: FrameId,
    /// Actual decoded source-frame dimensions, not configured dimensions.
    pub extent: PixelExtent,
    /// Frame pixels; fractional native content placement need not equal extent.
    pub content_bounds: Bounds,
    pub desktop_mapping: DesktopMapping,
}

impl AcquiredFrame {
    pub fn is_valid(&self) -> bool {
        PixelRect::covering(self.content_bounds, self.extent).is_some()
            && self.desktop_mapping.is_valid()
            && match self.desktop_mapping {
                DesktopMapping::Known {
                    desktop_bounds,
                    scale_x,
                    scale_y,
                    ..
                } => valid_bounds(Bounds {
                    x: self.content_bounds.x,
                    y: self.content_bounds.y,
                    width: desktop_bounds.width * scale_x,
                    height: desktop_bounds.height * scale_y,
                }),
                DesktopMapping::Unavailable { .. } => true,
            }
    }

    pub fn desktop_to_frame(&self, space: DesktopSpace, bounds: Bounds) -> Option<Bounds> {
        let DesktopMapping::Known {
            space: actual,
            desktop_bounds,
            scale_x,
            scale_y,
        } = self.desktop_mapping
        else {
            return None;
        };
        if space != actual || !valid_bounds(bounds) || !self.is_valid() {
            return None;
        }
        let mapped = Bounds {
            x: self.content_bounds.x + (bounds.x - desktop_bounds.x) * scale_x,
            y: self.content_bounds.y + (bounds.y - desktop_bounds.y) * scale_y,
            width: bounds.width * scale_x,
            height: bounds.height * scale_y,
        };
        valid_bounds(mapped).then_some(mapped)
    }

    pub fn clip(&self, bounds: Bounds) -> Option<Bounds> {
        if !self.is_valid() || !valid_bounds(bounds) {
            return None;
        }
        if bounds.x >= self.content_bounds.x.max(0.0)
            && bounds.y >= self.content_bounds.y.max(0.0)
            && bounds.x + bounds.width
                <= (self.content_bounds.x + self.content_bounds.width)
                    .min(f64::from(self.extent.width))
            && bounds.y + bounds.height
                <= (self.content_bounds.y + self.content_bounds.height)
                    .min(f64::from(self.extent.height))
        {
            return Some(bounds);
        }
        let x = bounds.x.max(self.content_bounds.x).max(0.0);
        let y = bounds.y.max(self.content_bounds.y).max(0.0);
        let right = (bounds.x + bounds.width)
            .min(self.content_bounds.x + self.content_bounds.width)
            .min(f64::from(self.extent.width));
        let bottom = (bounds.y + bounds.height)
            .min(self.content_bounds.y + self.content_bounds.height)
            .min(f64::from(self.extent.height));
        let clipped = Bounds {
            x,
            y,
            width: right - x,
            height: bottom - y,
        };
        valid_bounds(clipped).then_some(clipped)
    }

    /// Validate published source/capture/crop/encoded facts, before storing bytes.
    /// The caller separately decodes the PNG and matches its encoded extent.
    pub fn validates_image(
        &self,
        source: Bounds,
        captured: Bounds,
        crop: PixelRect,
        encoded: PixelExtent,
    ) -> bool {
        self.clip(source) == Some(captured)
            && PixelRect::covering(captured, self.extent) == Some(crop)
            && encoded.is_valid()
    }
}

pub fn valid_bounds(bounds: Bounds) -> bool {
    [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f64::is_finite)
        && bounds.width > 0.0
        && bounds.height > 0.0
        && (bounds.x + bounds.width).is_finite()
        && bounds.x + bounds.width > bounds.x
        && (bounds.y + bounds.height).is_finite()
        && bounds.y + bounds.height > bounds.y
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_frame_validates_required_geometry_without_restoring_live_authority() {
        let mut frame = AcquiredFrame {
            id: FrameId::try_from(Uuid::from_u128(1)).unwrap(),
            extent: PixelExtent {
                width: 100,
                height: 80,
            },
            content_bounds: Bounds {
                x: 2.25,
                y: 0.0,
                width: 97.75001,
                height: 80.00001,
            },
            desktop_mapping: DesktopMapping::Unavailable {
                reason: GeometryUnavailableReason::UnverifiedConfiguration,
            },
        };
        let captured = frame.clip(frame.content_bounds).unwrap();
        let crop = PixelRect::covering(captured, frame.extent).unwrap();
        assert!(frame.validates_image(
            frame.content_bounds,
            captured,
            crop,
            PixelExtent {
                width: 25,
                height: 20
            }
        ));
        let json = serde_json::to_value(&frame).unwrap();
        assert!(json.get("target").is_none());
        assert!(json.get("read").is_none());
        frame.desktop_mapping = DesktopMapping::Known {
            space: DesktopSpace::DesktopLogical,
            desktop_bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            },
            scale_x: f64::MAX,
            scale_y: 1.0,
        };
        assert!(!frame.is_valid());
    }
    #[test]
    fn crop_intersects_and_rounds_outward() {
        let extent = PixelExtent {
            width: 100,
            height: 80,
        };
        assert_eq!(
            PixelRect::covering(
                Bounds {
                    x: -0.2,
                    y: 1.2,
                    width: 100.4,
                    height: 4.1
                },
                extent
            ),
            Some(PixelRect {
                x: 0,
                y: 1,
                width: 100,
                height: 5
            })
        );
        assert!(PixelRect::covering(
            Bounds {
                x: 100.0,
                y: 0.0,
                width: 1.0,
                height: 1.0
            },
            extent
        )
        .is_none());
    }

    #[test]
    fn containment_preserves_fractional_source_and_coverage_identity() {
        let frame = AcquiredFrame {
            id: FrameId::try_from(Uuid::from_u128(1)).unwrap(),
            extent: PixelExtent {
                width: 100,
                height: 100,
            },
            content_bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            desktop_mapping: DesktopMapping::Unavailable {
                reason: GeometryUnavailableReason::MissingMetadata,
            },
        };
        let source = Bounds {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        };
        assert_eq!(frame.clip(source), Some(source));
    }
    #[test]
    fn rejects_collapsed_overflow_and_nil() {
        assert!(!valid_bounds(Bounds {
            x: f64::MAX,
            y: 0.0,
            width: 1.0,
            height: 1.0
        }));
        assert!(!valid_bounds(Bounds {
            x: 0.0,
            y: 0.0,
            width: f64::INFINITY,
            height: 1.0
        }));
        assert!(FrameId::try_from(Uuid::nil()).is_err());
        assert!(!PixelRect {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1
        }
        .within(PixelExtent {
            width: u32::MAX,
            height: 1
        }));
    }
}
