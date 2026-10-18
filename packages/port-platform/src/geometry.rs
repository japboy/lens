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

use crate::{capture::CaptureTarget, model::Bounds};
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
    /// A known transform must map the desktop window's full extent to the
    /// acquired content extent; finite components alone do not establish it.
    pub fn is_valid_for(&self, content: Bounds) -> bool {
        valid_bounds(content)
            && self.is_valid()
            && match *self {
                Self::Known {
                    desktop_bounds,
                    scale_x,
                    scale_y,
                    ..
                } => {
                    let mapped = Bounds {
                        x: content.x,
                        y: content.y,
                        width: desktop_bounds.width * scale_x,
                        height: desktop_bounds.height * scale_y,
                    };
                    valid_bounds(mapped)
                        && same_dimension(mapped.width, content.width)
                        && same_dimension(mapped.height, content.height)
                }
                Self::Unavailable { .. } => true,
            }
    }

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

#[derive(Debug, Clone, PartialEq)]
pub struct AcquiredFrame {
    pub id: FrameId,
    /// Must exactly equal the requested target/read before publication.
    pub target: CaptureTarget,
    /// Actual decoded source-frame dimensions, not configured dimensions.
    pub extent: PixelExtent,
    /// Frame pixels; fractional native content placement need not equal extent.
    pub content_bounds: Bounds,
    pub desktop_mapping: DesktopMapping,
}

impl AcquiredFrame {
    pub fn is_valid(&self) -> bool {
        PixelRect::covering(self.content_bounds, self.extent).is_some()
            && self.desktop_mapping.is_valid_for(self.content_bounds)
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

// Allow only f64 arithmetic roundoff in the few products composing a transform,
// not pixel/visual error. There is no absolute floor: for subnormal dimensions
// the tolerance can underflow to zero, requiring exact equality.
fn same_dimension(left: f64, right: f64) -> bool {
    (left - right).abs() <= 8.0 * f64::EPSILON * left.max(right)
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
    fn known_mapping_must_match_both_content_dimensions_with_only_numeric_roundoff() {
        let mut frame = AcquiredFrame {
            id: FrameId::try_from(Uuid::from_u128(1)).unwrap(),
            target: CaptureTarget::Legacy { window_id: 1 },
            extent: PixelExtent {
                width: 1200,
                height: 844,
            },
            content_bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 1200.0000095367432,
                height: 832.94118309021,
            },
            desktop_mapping: DesktopMapping::Known {
                space: DesktopSpace::DesktopLogical,
                desktop_bounds: Bounds {
                    x: -12.0,
                    y: 80.0,
                    width: 680.0,
                    height: 472.0,
                },
                scale_x: 1.7647058963775635,
                scale_y: 1.7647058963775635,
            },
        };
        // Recorded public-fixture content/scale values: fractional content may exceed
        // the encoded extent, but agrees with the scaled desktop dimensions.
        assert!(frame.is_valid());
        frame.content_bounds = Bounds {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 300.0,
        };
        frame.desktop_mapping = DesktopMapping::Known {
            space: DesktopSpace::DesktopLogical,
            desktop_bounds: Bounds {
                x: -12.0,
                y: 80.0,
                width: 100.0,
                height: 100.0,
            },
            scale_x: 2.0,
            scale_y: 3.0,
        };
        assert!(frame.is_valid(), "independent axis scales are supported");
        for axis in [0, 1] {
            let mut inconsistent = frame.clone();
            if let DesktopMapping::Known {
                scale_x, scale_y, ..
            } = &mut inconsistent.desktop_mapping
            {
                if axis == 0 {
                    *scale_x += 0.001;
                } else {
                    *scale_y += 0.001;
                }
            }
            assert!(!inconsistent.is_valid(), "axis {axis} contradiction");
        }
        let mut rounded = frame.clone();
        rounded.content_bounds.width = 200.0 * (1.0 + f64::EPSILON);
        assert!(rounded.is_valid());
        rounded.content_bounds.width = 200.0 + 1e-9;
        assert!(!rounded.is_valid(), "not a visual/pixel tolerance");
        frame.content_bounds.width = 1e-310;
        frame.content_bounds.height = 1e-310;
        frame.desktop_mapping = DesktopMapping::Known {
            space: DesktopSpace::DesktopLogical,
            desktop_bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 1e-310,
                height: 1e-310,
            },
            scale_x: 1.0,
            scale_y: 1.0,
        };
        assert!(frame.is_valid());
        frame.content_bounds.width = 2e-310;
        assert!(
            !frame.is_valid(),
            "tiny dimensions do not get an absolute tolerance floor"
        );
    }
    fn frame() -> AcquiredFrame {
        AcquiredFrame {
            id: FrameId::try_from(Uuid::from_u128(1)).unwrap(),
            target: CaptureTarget::Legacy { window_id: 1 },
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
            desktop_mapping: DesktopMapping::Known {
                space: DesktopSpace::DesktopLogical,
                desktop_bounds: Bounds {
                    x: -40.0,
                    y: 30.0,
                    width: 48.875005,
                    height: 40.000005,
                },
                scale_x: 2.0,
                scale_y: 2.0,
            },
        }
    }

    #[test]
    fn mapping_uses_sample_placement_and_rejects_other_desktop_units() {
        let frame = frame();
        let region = Bounds {
            x: -39.0,
            y: 31.0,
            width: 5.0,
            height: 4.0,
        };
        let mapped = frame
            .desktop_to_frame(DesktopSpace::DesktopLogical, region)
            .unwrap();
        assert_eq!(
            mapped,
            Bounds {
                x: 4.25,
                y: 2.0,
                width: 10.0,
                height: 8.0
            }
        );
        assert!(frame
            .desktop_to_frame(DesktopSpace::DesktopPhysicalPixels, region)
            .is_none());
        let crop = PixelRect::covering(mapped, frame.extent).unwrap();
        assert!(frame.validates_image(
            mapped,
            mapped,
            crop,
            PixelExtent {
                width: 5,
                height: 4
            }
        ));
        assert!(!frame.validates_image(
            mapped,
            mapped,
            PixelRect {
                width: crop.width - 1,
                ..crop
            },
            PixelExtent {
                width: 5,
                height: 4
            }
        ));
    }

    #[test]
    fn unavailable_mapping_keeps_frame_crop_but_overflowed_known_mapping_is_invalid() {
        let mut frame = frame();
        frame.desktop_mapping = DesktopMapping::Unavailable {
            reason: GeometryUnavailableReason::UnverifiedConfiguration,
        };
        assert!(frame.is_valid());
        assert!(frame.clip(frame.content_bounds).is_some());
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
        let frame = frame();
        let source = Bounds {
            x: 2.3,
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
