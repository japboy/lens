//! Explicit compatibility conversion between independent platform facts and domain values.
use domain::model as domain;
use port_platform::{model as platform, selection};
use serde::{Deserialize, Serialize};

/// Versioned shell contract, independent from native platform facts. This is not
/// part of the Agent document/context schemas and grants no target read authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessibilityAccessReply {
    pub schema_version: u32,
    #[serde(flatten)]
    pub availability: AccessibilityAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AccessibilityAvailability {
    Ready,
    PermissionRequired { action: AccessibilityAction },
    AccessRestricted,
    Unsupported,
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityAction {
    RequestPermission,
}

pub fn accessibility_access(
    access: port_platform::trust::AccessibilityAccess,
) -> AccessibilityAccessReply {
    use port_platform::trust::AccessibilityAccess;
    let availability = match access {
        AccessibilityAccess::Ready => AccessibilityAvailability::Ready,
        AccessibilityAccess::PermissionRequired => AccessibilityAvailability::PermissionRequired {
            action: AccessibilityAction::RequestPermission,
        },
        AccessibilityAccess::AccessRestricted => AccessibilityAvailability::AccessRestricted,
        AccessibilityAccess::Unsupported => AccessibilityAvailability::Unsupported,
        AccessibilityAccess::Failed { message } => AccessibilityAvailability::Failed { message },
    };
    AccessibilityAccessReply {
        schema_version: 1,
        availability,
    }
}

/// Re-inspect at the native command boundary. A stale WebView cannot turn a
/// restricted/unsupported/failed capability into a permission-prompt action.
pub fn request_accessibility_access(
    trust: &dyn port_platform::trust::AccessibilityTrust,
) -> AccessibilityAccessReply {
    use port_platform::trust::AccessibilityAccess;
    let access = match trust.inspect() {
        AccessibilityAccess::PermissionRequired => trust.request(),
        other => other,
    };
    accessibility_access(access)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WindowPickerReply {
    Selected {
        windows: Vec<domain::SelectedWindow>,
    },
    Cancelled,
    Error {
        message: String,
    },
}

impl WindowPickerReply {
    pub fn into_selected(self) -> Result<Option<Vec<domain::SelectedWindow>>, String> {
        match self {
            Self::Selected { windows } => Ok(Some(windows)),
            Self::Cancelled => Ok(None),
            Self::Error { message } => Err(message),
        }
    }
}

pub fn picker_reply(reply: selection::WindowPickerReply) -> WindowPickerReply {
    match reply {
        selection::WindowPickerReply::Selected { windows } => WindowPickerReply::Selected {
            windows: windows
                .into_iter()
                .map(selected_window_from_platform)
                .collect(),
        },
        selection::WindowPickerReply::Cancelled => WindowPickerReply::Cancelled,
        selection::WindowPickerReply::Error { message } => WindowPickerReply::Error { message },
    }
}

pub(crate) fn bounds_to_platform(bounds: domain::Bounds) -> platform::Bounds {
    platform::Bounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

pub(crate) fn bounds_from_platform(bounds: platform::Bounds) -> domain::Bounds {
    domain::Bounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

pub fn window_identity(
    identity: &domain::WindowIdentity,
) -> Result<platform::WindowIdentity, port_platform::PlatformError> {
    if identity.operation_id.is_nil() || identity.selection_ordinal == 0 {
        return Err(port_platform::PlatformError::Operation(
            "invalid target authority".into(),
        ));
    }
    Ok(platform::WindowIdentity {
        operation_id: identity.operation_id,
        receipt: identity
            .receipt
            .try_into()
            .map_err(|message: &str| port_platform::PlatformError::Operation(message.into()))?,
        selection_ordinal: identity.selection_ordinal,
    })
}

pub fn selected_window(
    window: &domain::SelectedWindow,
) -> Result<platform::SelectedWindow, port_platform::PlatformError> {
    Ok(platform::SelectedWindow {
        identity: window_identity(&window.identity)?,
        facts: platform::WindowObservableFacts {
            application_id: window.facts.application_id.clone(),
            title: window.facts.title.clone(),
            application_name: window.facts.application_name.clone(),
            frame: bounds_to_platform(window.facts.frame),
        },
    })
}

fn selected_window_from_platform(window: platform::SelectedWindow) -> domain::SelectedWindow {
    domain::SelectedWindow {
        identity: domain::WindowIdentity {
            operation_id: window.identity.operation_id,
            receipt: window.identity.receipt.into(),
            selection_ordinal: window.identity.selection_ordinal,
        },
        facts: facts_from_platform(window.facts),
    }
}

fn facts_from_platform(facts: platform::WindowObservableFacts) -> domain::WindowObservableFacts {
    domain::WindowObservableFacts {
        application_id: facts.application_id,
        title: facts.title,
        application_name: facts.application_name,
        frame: bounds_from_platform(facts.frame),
    }
}

fn semantic_kind_from_platform(kind: platform::SemanticKind) -> domain::SemanticKind {
    match kind {
        platform::SemanticKind::Heading => domain::SemanticKind::Heading,
        platform::SemanticKind::Text => domain::SemanticKind::Text,
        platform::SemanticKind::List => domain::SemanticKind::List,
        platform::SemanticKind::ListItem => domain::SemanticKind::ListItem,
        platform::SemanticKind::Table => domain::SemanticKind::Table,
        platform::SemanticKind::Row => domain::SemanticKind::Row,
        platform::SemanticKind::Cell => domain::SemanticKind::Cell,
        platform::SemanticKind::Link => domain::SemanticKind::Link,
        platform::SemanticKind::Control => domain::SemanticKind::Control,
        platform::SemanticKind::Dialog => domain::SemanticKind::Dialog,
        platform::SemanticKind::Region => domain::SemanticKind::Region,
        platform::SemanticKind::Paragraph => domain::SemanticKind::Paragraph,
        platform::SemanticKind::Image => domain::SemanticKind::Image,
        platform::SemanticKind::Unknown => domain::SemanticKind::Unknown,
    }
}

pub fn extraction(result: platform::ExtractionResult) -> domain::ExtractionResult {
    let metrics = result.metrics;
    domain::ExtractionResult {
        geometry: result.geometry.map(crate::geometry::descriptor),
        quality: match result.quality {
            platform::ExtractionQuality::Full => domain::ExtractionQuality::Full,
            platform::ExtractionQuality::Partial => domain::ExtractionQuality::Partial,
            platform::ExtractionQuality::Unavailable => domain::ExtractionQuality::Unavailable,
        },
        resolved_window: result.resolved_window.map(|window| domain::ResolvedWindow {
            facts: facts_from_platform(window.facts),
            resolution_score: window.resolution_score,
        }),
        nodes: result
            .nodes
            .into_iter()
            .map(|node| domain::ExtractedNode {
                id: node.id,
                parent_id: node.parent_id,
                order: node.order,
                depth: node.depth,
                source_api: match node.source_api {
                    platform::SourceApi::MacosAx => domain::SourceApi::MacosAx,
                    platform::SourceApi::WindowsUia => domain::SourceApi::WindowsUia,
                },
                semantic_kind: semantic_kind_from_platform(node.semantic_kind),
                node_purpose: match node.node_purpose {
                    platform::NodePurpose::Content => domain::NodePurpose::Content,
                    platform::NodePurpose::WindowChrome => domain::NodePurpose::WindowChrome,
                },
                native_role: node.native_role,
                native_subrole: node.native_subrole,
                title: node.title,
                value: node.value,
                description: node.description,
                bounds: node.bounds.map(crate::geometry::node_bounds),
                resource_refs: node
                    .resource_refs
                    .into_iter()
                    .map(|reference| domain::ResourceReference {
                        uri: reference.uri,
                        source_attribute: reference.source_attribute,
                    })
                    .collect(),
                children: node.children,
            })
            .collect(),
        text: result.text,
        metrics: domain::ExtractionMetrics {
            visited_nodes: metrics.visited_nodes,
            text_bytes: metrics.text_bytes,
            offscreen_text_nodes: metrics.offscreen_text_nodes,
            virtualization_signals: metrics.virtualization_signals,
            truncated_nodes: metrics.truncated_nodes,
            truncated_text: metrics.truncated_text,
            children_read_errors: metrics.children_read_errors,
            resource_ref_count: metrics.resource_ref_count,
            resource_uri_bytes: metrics.resource_uri_bytes,
            omitted_resource_refs: metrics.omitted_resource_refs,
            resource_read_errors: metrics.resource_read_errors,
        },
        diagnostics: result.diagnostics,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn access_contract_exposes_only_the_declared_permission_action() {
        use port_platform::trust::AccessibilityAccess as Access;
        use serde_json::json;
        for (access, expected) in [
            (Access::Ready, json!({"schema_version":1,"status":"ready"})),
            (
                Access::PermissionRequired,
                json!({"schema_version":1,"status":"permission_required","action":"request_permission"}),
            ),
            (
                Access::AccessRestricted,
                json!({"schema_version":1,"status":"access_restricted"}),
            ),
            (
                Access::Unsupported,
                json!({"schema_version":1,"status":"unsupported"}),
            ),
            (
                Access::Failed {
                    message: "inspection failed".into(),
                },
                json!({"schema_version":1,"status":"failed","message":"inspection failed"}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(super::accessibility_access(access)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn permission_request_rechecks_availability_before_any_effect() {
        use port_platform::trust::{AccessibilityAccess as Access, AccessibilityTrust};
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Trust {
            inspected: Access,
            requests: AtomicUsize,
        }
        impl AccessibilityTrust for Trust {
            fn inspect(&self) -> Access {
                self.inspected.clone()
            }
            fn request(&self) -> Access {
                self.requests.fetch_add(1, Ordering::SeqCst);
                Access::Ready
            }
        }
        for inspected in [
            Access::Ready,
            Access::PermissionRequired,
            Access::AccessRestricted,
            Access::Unsupported,
            Access::Failed {
                message: "unavailable".into(),
            },
        ] {
            let requires = inspected == Access::PermissionRequired;
            let expected = if requires {
                Access::Ready
            } else {
                inspected.clone()
            };
            let trust = Trust {
                inspected,
                requests: AtomicUsize::new(0),
            };
            assert_eq!(
                super::request_accessibility_access(&trust),
                super::accessibility_access(expected)
            );
            assert_eq!(trust.requests.load(Ordering::SeqCst), usize::from(requires));
        }
    }

    use super::*;
    use serde_json::{json, Value};

    fn window_fixture() -> Value {
        json!({
            "operation_id":uuid::Uuid::from_u128(1),"receipt":uuid::Uuid::from_u128(17),"selection_ordinal":1,"application_id":"com.example.fixture",
            "title":"Fixture window","application_name":"Fixture",
            "frame":{"x":1.5,"y":-2.0,"width":300.0,"height":200.0}
        })
    }

    #[test]
    fn picker_conversion_preserves_all_terminal_states_and_flattened_fields() {
        for value in [
            json!({"status":"selected","windows":[window_fixture()]}),
            json!({"status":"selected","windows":[window_fixture(),window_fixture()]}),
            json!({"status":"selected","windows":[]}),
            json!({"status":"cancelled"}),
            json!({"status":"error","message":"picker failed"}),
        ] {
            let native: selection::WindowPickerReply =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(picker_reply(native)).unwrap(), value);
        }
        assert_eq!(
            picker_reply(selection::WindowPickerReply::Cancelled).into_selected(),
            Ok(None)
        );
        assert_eq!(
            picker_reply(selection::WindowPickerReply::Error {
                message: "picker failed".into()
            })
            .into_selected(),
            Err("picker failed".into())
        );
    }

    #[test]
    fn invalid_domain_identity_returns_an_error_without_native_conversion() {
        let window: domain::SelectedWindow = serde_json::from_value(window_fixture()).unwrap();
        for axis in 0..3 {
            let mut invalid = window.clone();
            match axis {
                0 => invalid.identity.operation_id = uuid::Uuid::nil(),
                1 => invalid.identity.receipt = uuid::Uuid::nil(),
                2 => invalid.identity.selection_ordinal = 0,
                _ => unreachable!(),
            }
            assert!(window_identity(&invalid.identity).is_err());
            assert!(selected_window(&invalid).is_err());
        }
    }

    #[test]
    fn fixed_identity_and_current_facts_convert_without_reconstruction() {
        let window: domain::SelectedWindow = serde_json::from_value(window_fixture()).unwrap();
        let native = selected_window(&window).unwrap();
        assert_eq!(serde_json::to_value(&native).unwrap(), window_fixture());
        assert_eq!(selected_window_from_platform(native), window);
        assert_eq!(
            serde_json::to_value(window_identity(&window.identity).unwrap()).unwrap(),
            json!({
                "operation_id":uuid::Uuid::from_u128(1),"receipt":uuid::Uuid::from_u128(17),"selection_ordinal":1
            })
        );
    }

    #[test]
    fn extraction_conversion_preserves_every_graph_field_and_diagnostic() {
        let value = json!({
            "quality":"partial",
            "resolved_window": {
                "facts": {"title":"Changed title","application_name":"Fixture","application_id":"com.example.fixture","frame":{
                    "x":20.0,"y":30.0,"width":350.0,"height":250.0
                }},
                "resolution_score":0.75
            },
            "nodes":[{
                "id":"node-2","parent_id":"node-1","order":2,"depth":1,
                "source_api":"macos_ax","semantic_kind":"image","node_purpose":"content",
                "native_role":"AXImage","native_subrole":"AXUnknownSubrole","title":"Chart",
                "value":"42","description":"Description",
                "bounds":{"kind":"legacy","geometry":{"frame":"macos_desktop_points","rect":{"x":21.0,"y":32.0,"width":10.0,"height":20.0}}},
                "resource_refs":[{"uri":"https://example.com/image","source_attribute":"AXURL"}],
                "children":["node-3"]
            }],
            "text":"Diagnostic compatibility text",
            "metrics":{
                "visited_nodes":11,"text_bytes":12,"offscreen_text_nodes":13,
                "virtualization_signals":14,"truncated_nodes":true,"truncated_text":true,
                "children_read_errors":15,"resource_ref_count":16,"resource_uri_bytes":17,
                "omitted_resource_refs":18,"resource_read_errors":19
            },
            "diagnostics":["first diagnostic","second diagnostic"]
        });
        let native: platform::ExtractionResult = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(extraction(native)).unwrap(), value);
    }

    #[test]
    fn role_facts_convert_without_native_string_reinterpretation() {
        for source_api in ["macos_ax", "windows_uia"] {
            for node_purpose in ["content", "window_chrome"] {
                for semantic_kind in [
                    "heading",
                    "text",
                    "list",
                    "list_item",
                    "table",
                    "row",
                    "cell",
                    "link",
                    "control",
                    "dialog",
                    "region",
                    "paragraph",
                    "image",
                    "unknown",
                ] {
                    let value = json!({"quality":"full","nodes":[{
                        "id":"root","order":0,"depth":0,
                        "source_api":source_api,"node_purpose":node_purpose,
                        "semantic_kind":semantic_kind,"native_role":"AXImage",
                        "native_subrole":"opaque-provider-value"
                    }]});
                    let port = serde_json::from_value::<platform::ExtractionResult>(value.clone())
                        .unwrap();
                    let expected =
                        serde_json::from_value::<domain::ExtractionResult>(value).unwrap();
                    assert_eq!(extraction(port), expected);
                }
            }
        }
    }

    #[test]
    fn missing_optional_extraction_fields_and_quality_variants_keep_defaults() {
        for quality in ["full", "partial", "unavailable"] {
            let value = json!({"quality":quality,"nodes":[{
                "id":"node-1","order":0,"depth":0,
                "source_api":"windows_uia","semantic_kind":"unknown","node_purpose":"content"
            }]});
            let native: platform::ExtractionResult = serde_json::from_value(value.clone()).unwrap();
            let expected: domain::ExtractionResult = serde_json::from_value(value).unwrap();
            assert_eq!(extraction(native), expected);
        }
    }
}
