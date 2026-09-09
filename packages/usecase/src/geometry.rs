//! Explicit conversion of independent platform facts to canonical domain values.
use domain::geometry as canonical;
use port_platform::{authority as source_authority, geometry as source};

fn authority(value: source_authority::TargetAuthority) -> canonical::TargetAuthority {
    canonical::TargetAuthority {
        operation_id: value.operation_id,
        receipt: uuid::Uuid::from(value.receipt)
            .try_into()
            .expect("port receipt is non-nil"),
    }
}

fn read(value: source_authority::TargetReadKey) -> canonical::TargetReadKey {
    canonical::TargetReadKey {
        target: authority(value.target),
        sequence: String::from(value.sequence)
            .try_into()
            .expect("port sequence is canonical nonzero u64"),
    }
}

fn capture(value: source::CaptureKey) -> canonical::CaptureKey {
    canonical::CaptureKey {
        read: read(value.read),
        capture_id: value.capture_id,
    }
}

fn frame(value: source::CoordinateFrame) -> canonical::CoordinateFrame {
    match value {
        source::CoordinateFrame::MacosDesktopPoints => {
            canonical::CoordinateFrame::MacosDesktopPoints
        }
        source::CoordinateFrame::WindowsDesktopPhysicalPixels => {
            canonical::CoordinateFrame::WindowsDesktopPhysicalPixels
        }
        source::CoordinateFrame::TargetLogical { target } => {
            canonical::CoordinateFrame::TargetLogical {
                target: authority(target),
            }
        }
        source::CoordinateFrame::OriginalCapturePixels { capture: key } => {
            canonical::CoordinateFrame::OriginalCapturePixels {
                capture: capture(key),
            }
        }
        source::CoordinateFrame::CroppedAttachmentPixels {
            capture: key,
            attachment_id,
        } => canonical::CoordinateFrame::CroppedAttachmentPixels {
            capture: capture(key),
            attachment_id,
        },
        source::CoordinateFrame::EncodedAttachmentPixels {
            capture: key,
            attachment_id,
        } => canonical::CoordinateFrame::EncodedAttachmentPixels {
            capture: capture(key),
            attachment_id,
        },
    }
}

pub(crate) fn descriptor(
    value: source::ReadGeometryDescriptor,
) -> canonical::ReadGeometryDescriptor {
    canonical::ReadGeometryDescriptor {
        read: read(value.read),
        window: canonical::TaggedRect {
            read: read(value.window.read),
            frame: frame(value.window.frame),
            rect: canonical::Rect {
                x: value.window.rect.x,
                y: value.window.rect.y,
                width: value.window.rect.width,
                height: value.window.rect.height,
            },
        },
        desktop_to_target: canonical::AxisAlignedTransform {
            read: read(value.desktop_to_target.read),
            source: frame(value.desktop_to_target.source),
            destination: frame(value.desktop_to_target.destination),
            scale_x: value.desktop_to_target.scale_x,
            scale_y: value.desktop_to_target.scale_y,
            translate_x: value.desktop_to_target.translate_x,
            translate_y: value.desktop_to_target.translate_y,
        },
    }
}

pub(crate) fn node_bounds(value: source::NodeBounds) -> canonical::NodeBounds {
    let rect = |value: source::Rect| canonical::Rect {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    };
    match value {
        source::NodeBounds::Registered { geometry } => canonical::NodeBounds::Registered {
            geometry: canonical::TaggedRect {
                read: read(geometry.read),
                frame: frame(geometry.frame),
                rect: rect(geometry.rect),
            },
        },
        source::NodeBounds::Legacy { geometry } => canonical::NodeBounds::Legacy {
            geometry: canonical::DesktopRect {
                frame: match geometry.frame {
                    source::DesktopFrame::MacosDesktopPoints => {
                        canonical::DesktopFrame::MacosDesktopPoints
                    }
                    source::DesktopFrame::WindowsDesktopPhysicalPixels => {
                        canonical::DesktopFrame::WindowsDesktopPhysicalPixels
                    }
                },
                rect: rect(geometry.rect),
            },
        },
    }
}

pub(crate) fn attachment(
    key: source::CaptureKey,
    attachment_id: String,
    pixels: source::CapturedPixelGeometry,
) -> Result<canonical::AttachmentGeometry, canonical::GeometryError> {
    let capture = capture(key);
    let extent = |value: source::PixelExtent| canonical::PixelExtent {
        width: value.width,
        height: value.height,
    };
    let cropped = canonical::CoordinateFrame::CroppedAttachmentPixels {
        capture,
        attachment_id: attachment_id.clone(),
    };
    let value = canonical::AttachmentGeometry {
        capture,
        attachment_id: attachment_id.clone(),
        original_extent: extent(pixels.original_extent),
        crop: canonical::PixelCrop {
            x: pixels.crop.x,
            y: pixels.crop.y,
            extent: extent(pixels.crop.extent),
        },
        encoded_extent: extent(pixels.encoded_extent),
        original_to_crop: canonical::AxisAlignedTransform {
            read: capture.read,
            source: canonical::CoordinateFrame::OriginalCapturePixels { capture },
            destination: cropped.clone(),
            scale_x: 1.0,
            scale_y: 1.0,
            translate_x: -f64::from(pixels.crop.x),
            translate_y: -f64::from(pixels.crop.y),
        },
        crop_to_encoded: canonical::AxisAlignedTransform {
            read: capture.read,
            source: cropped,
            destination: canonical::CoordinateFrame::EncodedAttachmentPixels {
                capture,
                attachment_id,
            },
            scale_x: f64::from(pixels.encoded_extent.width) / f64::from(pixels.crop.extent.width),
            scale_y: f64::from(pixels.encoded_extent.height) / f64::from(pixels.crop.extent.height),
            translate_x: 0.0,
            translate_y: 0.0,
        },
    };
    value.validate()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_conversion_preserves_exact_wire_and_validation() {
        let key = source_authority::TargetReadKey {
            target: source_authority::TargetAuthority {
                operation_id: uuid::Uuid::from_u128(1),
                receipt: uuid::Uuid::from_u128(2).try_into().unwrap(),
            },
            sequence: u64::MAX.to_string().try_into().unwrap(),
        };
        let value = source::ReadGeometryDescriptor {
            read: key,
            window: source::TaggedRect {
                read: key,
                frame: source::CoordinateFrame::WindowsDesktopPhysicalPixels,
                rect: source::Rect {
                    x: -120.5,
                    y: 40.25,
                    width: 800.0,
                    height: 600.0,
                },
            },
            desktop_to_target: source::AxisAlignedTransform {
                read: key,
                source: source::CoordinateFrame::WindowsDesktopPhysicalPixels,
                destination: source::CoordinateFrame::TargetLogical { target: key.target },
                scale_x: 0.5,
                scale_y: 0.25,
                translate_x: 60.25,
                translate_y: -10.0625,
            },
        };
        value.validate().unwrap();
        let converted = descriptor(value.clone());
        converted.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            serde_json::to_value(&converted).unwrap()
        );
        assert_eq!(String::from(converted.read.sequence), u64::MAX.to_string());
    }
}
