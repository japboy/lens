//! Explicit compatibility conversion between independent platform facts and domain values.
use domain::model as domain;
use port_platform::{model as platform, selection};
use serde::{Deserialize, Serialize};

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

pub fn window_identity(identity: &domain::WindowIdentity) -> platform::WindowIdentity {
    platform::WindowIdentity {
        window_id: identity.window_id,
        bundle_id: identity.bundle_id.clone(),
        pid: identity.pid,
    }
}

pub fn selected_window(window: &domain::SelectedWindow) -> platform::SelectedWindow {
    platform::SelectedWindow {
        identity: window_identity(&window.identity),
        facts: platform::WindowObservableFacts {
            title: window.facts.title.clone(),
            application_name: window.facts.application_name.clone(),
            frame: bounds_to_platform(window.facts.frame),
        },
    }
}

fn selected_window_from_platform(window: platform::SelectedWindow) -> domain::SelectedWindow {
    domain::SelectedWindow {
        identity: domain::WindowIdentity {
            window_id: window.identity.window_id,
            bundle_id: window.identity.bundle_id,
            pid: window.identity.pid,
        },
        facts: facts_from_platform(window.facts),
    }
}

fn facts_from_platform(facts: platform::WindowObservableFacts) -> domain::WindowObservableFacts {
    domain::WindowObservableFacts {
        title: facts.title,
        application_name: facts.application_name,
        frame: bounds_from_platform(facts.frame),
    }
}

pub fn extraction(result: platform::ExtractionResult) -> domain::ExtractionResult {
    let metrics = result.metrics;
    domain::ExtractionResult {
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
                role: node.role,
                subrole: node.subrole,
                title: node.title,
                value: node.value,
                description: node.description,
                bounds: node.bounds.map(bounds_from_platform),
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
    use super::*;
    use serde_json::{json, Value};

    fn window_fixture() -> Value {
        json!({
            "window_id":17,"bundle_id":"com.example.fixture","pid":42,
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
    fn fixed_identity_and_current_facts_convert_without_reconstruction() {
        let window: domain::SelectedWindow = serde_json::from_value(window_fixture()).unwrap();
        let native = selected_window(&window);
        assert_eq!(serde_json::to_value(&native).unwrap(), window_fixture());
        assert_eq!(selected_window_from_platform(native), window);
        assert_eq!(
            serde_json::to_value(window_identity(&window.identity)).unwrap(),
            json!({
                "window_id":17,"bundle_id":"com.example.fixture","pid":42
            })
        );
    }

    #[test]
    fn extraction_conversion_preserves_every_graph_field_and_diagnostic() {
        let value = json!({
            "quality":"partial",
            "resolved_window": {
                "facts": {"title":"Changed title","application_name":"Fixture","frame":{
                    "x":20.0,"y":30.0,"width":350.0,"height":250.0
                }},
                "resolution_score":0.75
            },
            "nodes":[{
                "id":"node-2","parent_id":"node-1","order":2,"depth":1,
                "role":"AXImage","subrole":"AXUnknownSubrole","title":"Chart",
                "value":"42","description":"Description",
                "bounds":{"x":21.0,"y":32.0,"width":10.0,"height":20.0},
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
    fn missing_optional_extraction_fields_and_quality_variants_keep_defaults() {
        for quality in ["full", "partial", "unavailable"] {
            let value = json!({"quality":quality,"nodes":[{"id":"node-1","order":0,"depth":0}]});
            let native: platform::ExtractionResult = serde_json::from_value(value.clone()).unwrap();
            let expected: domain::ExtractionResult = serde_json::from_value(value).unwrap();
            assert_eq!(extraction(native), expected);
        }
    }
}
