//! Private native endpoint observations, independent of domain and publication state.
use port_platform::{
    authority::TargetReadKey,
    geometry::{AxisAlignedTransform, CoordinateFrame, ReadGeometryDescriptor, Rect, TaggedRect},
    model::Bounds,
    PlatformError,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeGeometryObservation {
    before: Bounds,
    after: Bounds,
}

pub(super) fn observed_descriptor(
    observation: Option<NativeGeometryObservation>,
    read: TargetReadKey,
    window: Option<Bounds>,
) -> Result<ReadGeometryDescriptor, PlatformError> {
    let observation = observation.ok_or_else(|| {
        PlatformError::InvalidResponse("registered acquisition has no geometry observation".into())
    })?;
    if observation.before != observation.after || window != Some(observation.before) {
        return Err(PlatformError::InvalidResponse(
            "acquisition geometry changed or disagrees with observed window".into(),
        ));
    }
    let bounds = observation.before;
    let descriptor = ReadGeometryDescriptor {
        read,
        window: TaggedRect {
            read,
            frame: CoordinateFrame::MacosDesktopPoints,
            rect: Rect {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
        },
        // Desktop points and target-local points share units; this does not infer DPI.
        desktop_to_target: AxisAlignedTransform {
            read,
            source: CoordinateFrame::MacosDesktopPoints,
            destination: CoordinateFrame::TargetLogical {
                target: read.target,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            translate_x: -bounds.x,
            translate_y: -bounds.y,
        },
    };
    descriptor
        .validate()
        .map_err(|error| PlatformError::InvalidResponse(error.to_string()))?;
    Ok(descriptor)
}
