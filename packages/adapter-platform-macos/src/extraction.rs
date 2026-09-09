use crate::geometry::NativeGeometryObservation;
use port_platform::geometry::{
    DesktopFrame, DesktopRect, NodeBounds, ReadGeometryDescriptor, Rect, TaggedRect,
};
use port_platform::model::{
    Bounds, ExtractedNode, ExtractionMetrics, ExtractionQuality, ExtractionResult, NodePurpose,
    ResolvedWindow, ResourceReference, SemanticKind, SourceApi,
};
use port_platform::PlatformError;
use serde::Deserialize;

// These DTOs describe the unchanged, private Objective-C transfer format.
#[derive(Deserialize)]
pub(super) struct NativeExtractionResult {
    #[serde(default)]
    read: Option<port_platform::authority::TargetReadKey>,
    geometry_observation: Option<NativeGeometryObservation>,
    quality: ExtractionQuality,
    #[serde(default)]
    resolved_window: Option<ResolvedWindow>,
    #[serde(default)]
    nodes: Vec<NativeExtractedNode>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    metrics: ExtractionMetrics,
    #[serde(default)]
    diagnostics: Vec<String>,
}

#[derive(Deserialize)]
struct NativeExtractedNode {
    id: String,
    #[serde(default)]
    parent_id: Option<String>,
    order: usize,
    depth: usize,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    subrole: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    bounds: Option<Bounds>,
    #[serde(default)]
    resource_refs: Vec<ResourceReference>,
    #[serde(default)]
    children: Vec<String>,
}

impl TryFrom<NativeExtractionResult> for ExtractionResult {
    type Error = PlatformError;

    fn try_from(value: NativeExtractionResult) -> Result<Self, Self::Error> {
        let geometry = match value.read {
            Some(read)
                if value.quality != ExtractionQuality::Unavailable
                    || value.resolved_window.is_some()
                    || !value.nodes.is_empty() =>
            {
                Some(crate::geometry::observed_descriptor(
                    value.geometry_observation,
                    read,
                    value
                        .resolved_window
                        .as_ref()
                        .map(|window| window.facts.frame),
                )?)
            }
            _ => None,
        };
        let mut omitted_bounds = 0usize;
        let nodes = value
            .nodes
            .into_iter()
            .map(|node| {
                let had_bounds = node.bounds.is_some();
                let converted = convert_node(node, value.read, geometry.as_ref())?;
                if had_bounds && converted.bounds.is_none() {
                    omitted_bounds += 1;
                }
                Ok::<_, PlatformError>(converted)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut diagnostics = value.diagnostics;
        if omitted_bounds != 0 {
            // One bounded summary, not an unbounded diagnostic per node or source text.
            diagnostics.push(format!(
                "Omitted invalid rectangles from {omitted_bounds} native nodes."
            ));
        }
        Ok(Self {
            read: value.read,
            geometry,
            quality: value.quality,
            resolved_window: value.resolved_window,
            nodes,
            text: value.text,
            metrics: value.metrics,
            diagnostics,
        })
    }
}

fn convert_node(
    value: NativeExtractedNode,
    read: Option<port_platform::authority::TargetReadKey>,
    descriptor: Option<&ReadGeometryDescriptor>,
) -> Result<ExtractedNode, PlatformError> {
    let bounds = value
        .bounds
        .map(|bounds| {
            // Authority errors must not be hidden by an unusable element rectangle.
            match (read, descriptor) {
                (Some(read), Some(descriptor)) if read == descriptor.read => {
                    descriptor
                        .validate()
                        .map_err(|error| PlatformError::Operation(error.to_string()))?;
                }
                (None, None) => {}
                _ => {
                    return Err(PlatformError::Operation(
                        "native node bounds require matching acquisition geometry".into(),
                    ))
                }
            }
            let rect = Rect {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            };
            if rect.validate().is_err() {
                return Ok(None);
            }
            let geometry = match (read, descriptor) {
                (Some(read), Some(descriptor)) if read == descriptor.read => {
                    NodeBounds::Registered {
                        geometry: TaggedRect {
                            read,
                            frame: descriptor.window.frame.clone(),
                            rect,
                        },
                    }
                }
                (None, None) => NodeBounds::Legacy {
                    geometry: DesktopRect {
                        frame: DesktopFrame::MacosDesktopPoints,
                        rect,
                    },
                },
                _ => {
                    return Err(PlatformError::Operation(
                        "native node bounds require matching acquisition geometry".into(),
                    ))
                }
            };
            geometry
                .validate_for(descriptor)
                .map_err(|error| PlatformError::Operation(error.to_string()))?;
            Ok(Some(geometry))
        })
        .transpose()?
        .flatten();
    Ok(ExtractedNode {
        semantic_kind: semantic_kind(value.role.as_deref()),
        node_purpose: if value.role.as_deref() == Some("AXWindow") {
            NodePurpose::WindowChrome
        } else {
            NodePurpose::Content
        },
        source_api: SourceApi::MacosAx,
        id: value.id,
        parent_id: value.parent_id,
        order: value.order,
        depth: value.depth,
        native_role: value.role,
        native_subrole: value.subrole,
        title: value.title,
        value: value.value,
        description: value.description,
        bounds,
        resource_refs: value.resource_refs,
        children: value.children,
    })
}

fn semantic_kind(role: Option<&str>) -> SemanticKind {
    match role.unwrap_or_default().to_ascii_lowercase().as_str() {
        "axheading" => SemanticKind::Heading,
        "axstatictext" | "axtext" => SemanticKind::Text,
        "axlist" => SemanticKind::List,
        "axlistitem" => SemanticKind::ListItem,
        "axtable" | "axoutline" => SemanticKind::Table,
        "axrow" => SemanticKind::Row,
        "axcell" | "axcolumn" => SemanticKind::Cell,
        "axlink" => SemanticKind::Link,
        "axbutton" | "axcheckbox" | "axradiobutton" | "axtextfield" | "axtextarea"
        | "axcombobox" | "axpopupbutton" | "axslider" | "axswitch" => SemanticKind::Control,
        "axdialog" | "axsheet" => SemanticKind::Dialog,
        "axgroup" | "axsection" | "axlandmark" | "axwebarea" | "axwindow" => SemanticKind::Region,
        "axparagraph" => SemanticKind::Paragraph,
        "aximage" => SemanticKind::Image,
        _ => SemanticKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use port_platform::geometry::CoordinateFrame;

    #[test]
    fn native_raw_node_bounds_gain_only_explicit_registered_or_legacy_authority() {
        let frame = serde_json::json!({"x":-100.25,"y":20.5,"width":800.0,"height":600.0});
        let mut wire = serde_json::json!({
            "read":{"target":{"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000002"},"sequence":"1"},
            "quality":"full",
            "resolved_window":{"facts":{"application_name":"Fixture","application_id":"example.fixture","title":"Fixture","frame":frame},"resolution_score":1.0},
            "geometry_observation":{"before":frame,"after":frame},
            "nodes":[{"id":"image","order":0,"depth":0,"role":"AXImage","bounds":{"x":-90.25,"y":30.5,"width":20.0,"height":10.0}}]
        });
        let decode = |value| {
            ExtractionResult::try_from(
                serde_json::from_value::<NativeExtractionResult>(value).unwrap(),
            )
        };
        let registered = decode(wire.clone()).unwrap();
        let bounds = registered.nodes[0].bounds.as_ref().unwrap();
        assert!(matches!(bounds, NodeBounds::Registered { .. }));
        assert_eq!(bounds.validate_for(registered.geometry.as_ref()), Ok(()));
        let descriptor = registered.geometry.unwrap();
        let node =
            || serde_json::from_value::<NativeExtractedNode>(wire["nodes"][0].clone()).unwrap();
        assert!(convert_node(node(), Some(descriptor.read), None).is_err());
        let mut wrong_read = descriptor.read;
        wrong_read.sequence = wrong_read.sequence.checked_next().unwrap();
        assert!(convert_node(node(), Some(wrong_read), Some(&descriptor)).is_err());
        wire["read"] = serde_json::Value::Null;
        wire["geometry_observation"] = serde_json::Value::Null;
        let legacy = decode(wire.clone()).unwrap();
        assert!(
            matches!(&legacy.nodes[0].bounds, Some(NodeBounds::Legacy { geometry }) if geometry.frame == DesktopFrame::MacosDesktopPoints)
        );
        for width in [0.0, -1.0] {
            wire["nodes"][0]["bounds"]["width"] = serde_json::json!(width);
            let result = decode(wire.clone()).unwrap();
            assert!(result.nodes[0].bounds.is_none());
            assert_eq!(
                result.diagnostics,
                ["Omitted invalid rectangles from 1 native nodes."]
            );
        }
        wire["nodes"][0]["bounds"] = serde_json::Value::Null;
        assert!(decode(wire).unwrap().nodes[0].bounds.is_none());
    }

    #[test]
    fn unusable_node_rectangles_preserve_text_and_do_not_hide_authority_errors() {
        let frame = serde_json::json!({"x":0.0,"y":0.0,"width":100.0,"height":100.0});
        let native: NativeExtractionResult = serde_json::from_value(serde_json::json!({
            "read":{"target":{"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000002"},"sequence":"1"},
            "quality":"full","text":"retained text",
            "resolved_window":{"facts":{"application_name":"Fixture","application_id":"example.fixture","title":"Fixture","frame":frame},"resolution_score":1.0},
            "geometry_observation":{"before":frame,"after":frame},
            "nodes":[
                {"id":"text","order":0,"depth":0,"role":"AXStaticText","value":"retained node text","bounds":{"x":0,"y":0,"width":0,"height":1}},
                {"id":"other","order":1,"depth":0,"bounds":{"x":1,"y":1,"width":10,"height":10}}
            ]
        })).unwrap();
        let result = ExtractionResult::try_from(native).unwrap();
        assert_eq!(result.text, "retained text");
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.nodes[0].value.as_deref(), Some("retained node text"));
        assert!(result.nodes[0].bounds.is_none());
        assert!(result.nodes[1].bounds.is_some());
        assert_eq!(
            result.diagnostics,
            ["Omitted invalid rectangles from 1 native nodes."]
        );
        let descriptor = result.geometry.unwrap();
        for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let node = || NativeExtractedNode {
                id: "text".into(),
                parent_id: None,
                order: 0,
                depth: 0,
                role: Some("AXStaticText".into()),
                subrole: None,
                title: None,
                value: Some("retained".into()),
                description: None,
                bounds: Some(Bounds {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: 1.0,
                }),
                resource_refs: vec![],
                children: vec![],
            };
            assert!(
                convert_node(node(), Some(descriptor.read), Some(&descriptor))
                    .unwrap()
                    .bounds
                    .is_none()
            );
            assert!(convert_node(node(), Some(descriptor.read), None).is_err());
            let mut wrong_read = descriptor.read;
            wrong_read.sequence = wrong_read.sequence.checked_next().unwrap();
            assert!(convert_node(node(), Some(wrong_read), Some(&descriptor)).is_err());
        }
    }

    #[test]
    fn registered_geometry_requires_matching_observed_frames() {
        let frame = serde_json::json!({"x":-100.25,"y":20.5,"width":800.0,"height":600.0});
        let wire = serde_json::json!({
            "read":{"target":{"operation_id":"00000000-0000-0000-0000-000000000001","receipt":"00000000-0000-0000-0000-000000000002"},"sequence":"1"},
            "quality":"full",
            "resolved_window":{"facts":{"application_name":"Fixture","application_id":"example.fixture","title":"Fixture","frame":frame},"resolution_score":1.0},
            "geometry_observation":{"before":frame,"after":frame},
            "nodes":[]
        });
        let decode = |wire| {
            ExtractionResult::try_from(
                serde_json::from_value::<NativeExtractionResult>(wire).unwrap(),
            )
        };
        let result = decode(wire.clone()).unwrap();
        let geometry = result.geometry.unwrap();
        assert_eq!(geometry.read, result.read.unwrap());
        assert_eq!(geometry.window.frame, CoordinateFrame::MacosDesktopPoints);
        let local = geometry.desktop_to_target.apply(&geometry.window).unwrap();
        assert_eq!(local.rect.x, 0.0);
        assert_eq!(local.rect.y, 0.0);
        assert_eq!(local.rect.width, 800.0);
        for path in ["before", "after"] {
            let mut changed = wire.clone();
            changed["geometry_observation"][path]["x"] = serde_json::json!(-99.0);
            assert!(decode(changed).is_err());
        }
        let mut missing = wire.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("geometry_observation");
        assert!(decode(missing).is_err());
        let mut invalid = wire.clone();
        for path in ["before", "after"] {
            invalid["geometry_observation"][path]["width"] = serde_json::json!(0.0);
        }
        invalid["resolved_window"]["facts"]["frame"]["width"] = serde_json::json!(0.0);
        assert!(decode(invalid).is_err());
        let mut unavailable = wire;
        unavailable["quality"] = serde_json::json!("unavailable");
        unavailable["geometry_observation"] = serde_json::Value::Null;
        assert!(decode(unavailable.clone()).is_err());
        unavailable["resolved_window"] = serde_json::Value::Null;
        assert!(decode(unavailable.clone()).unwrap().geometry.is_none());
        unavailable["nodes"] = serde_json::json!([
            {"id":"image","order":0,"depth":0,"role":"AXImage"}
        ]);
        assert!(decode(unavailable).is_err());
    }

    #[test]
    fn ax_mapping_preserves_every_existing_kind_and_ascii_case_rule() {
        let cases = [
            ("AXHeading", SemanticKind::Heading),
            ("AXStaticText", SemanticKind::Text),
            ("AXText", SemanticKind::Text),
            ("AXList", SemanticKind::List),
            ("AXListItem", SemanticKind::ListItem),
            ("AXTable", SemanticKind::Table),
            ("AXOutline", SemanticKind::Table),
            ("AXRow", SemanticKind::Row),
            ("AXCell", SemanticKind::Cell),
            ("AXColumn", SemanticKind::Cell),
            ("AXLink", SemanticKind::Link),
            ("AXButton", SemanticKind::Control),
            ("AXCheckBox", SemanticKind::Control),
            ("AXRadioButton", SemanticKind::Control),
            ("AXTextField", SemanticKind::Control),
            ("AXTextArea", SemanticKind::Control),
            ("AXComboBox", SemanticKind::Control),
            ("AXPopUpButton", SemanticKind::Control),
            ("AXSlider", SemanticKind::Control),
            ("AXSwitch", SemanticKind::Control),
            ("AXDialog", SemanticKind::Dialog),
            ("AXSheet", SemanticKind::Dialog),
            ("AXGroup", SemanticKind::Region),
            ("AXSection", SemanticKind::Region),
            ("AXLandmark", SemanticKind::Region),
            ("AXWebArea", SemanticKind::Region),
            ("AXWindow", SemanticKind::Region),
            ("AXParagraph", SemanticKind::Paragraph),
            ("AXImage", SemanticKind::Image),
        ];
        for (role, kind) in cases {
            assert_eq!(semantic_kind(Some(role)), kind, "{role}");
            assert_eq!(semantic_kind(Some(&role.to_ascii_uppercase())), kind);
            assert_eq!(semantic_kind(Some(&role.to_ascii_lowercase())), kind);
        }
        for role in [
            None,
            Some(""),
            Some(" AXImage"),
            Some("UIA_ImageControlTypeId"),
            Some("AXFuture"),
        ] {
            assert_eq!(semantic_kind(role), SemanticKind::Unknown);
        }
    }

    #[test]
    fn native_transfer_keeps_raw_roles_and_optional_defaults() {
        let native: NativeExtractionResult = serde_json::from_value(serde_json::json!({
            "quality": "full",
            "nodes": [
                {"id": "one", "order": 0, "depth": 0, "role": "AXFuture", "subrole": "Future"},
                {"id": "two", "order": 1, "depth": 0},
                {"id": "three", "order": 2, "depth": 0, "role": "AXWindow"},
                {"id": "four", "order": 3, "depth": 0, "role": "axwindow"}
            ]
        }))
        .unwrap();
        let result = ExtractionResult::try_from(native).unwrap();
        assert_eq!(result.nodes[0].native_role.as_deref(), Some("AXFuture"));
        assert_eq!(result.nodes[0].native_subrole.as_deref(), Some("Future"));
        assert_eq!(result.nodes[0].semantic_kind, SemanticKind::Unknown);
        assert_eq!(result.nodes[1].native_role, None);
        assert_eq!(result.nodes[1].native_subrole, None);
        assert_eq!(result.nodes[1].semantic_kind, SemanticKind::Unknown);
        assert_eq!(result.nodes[2].node_purpose, NodePurpose::WindowChrome);
        assert_eq!(result.nodes[3].node_purpose, NodePurpose::Content);
        assert_eq!(result.nodes[2].semantic_kind, SemanticKind::Region);
        assert_eq!(result.nodes[3].semantic_kind, SemanticKind::Region);
        assert!(result
            .nodes
            .iter()
            .all(|node| node.source_api == SourceApi::MacosAx));
    }
}
