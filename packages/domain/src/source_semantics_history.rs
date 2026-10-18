//! Explicit migration of the last AX-only source schemas. Fresh extraction has
//! no compatibility decoder; native names are interpreted here only as archived
//! facts from these specific historical macOS contracts.
use crate::{lens::*, model::*};
use serde::{de::Error, Deserialize, Deserializer};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

fn version(value: &Value, old: u64, current: u64) -> Result<bool, String> {
    match value.get("schema_version").and_then(Value::as_u64) {
        Some(v) if v == old => Ok(true),
        Some(v) if v == current => Ok(false),
        _ => Err(format!(
            "unsupported source schema; expected {old} or {current}"
        )),
    }
}

fn historical_purpose(node: &Value) -> &'static str {
    match node.get("role").and_then(Value::as_str) {
        Some("AXWindow") => "window_root",
        Some(role)
            if ["AXTextField", "AXTextArea", "AXComboBox"]
                .iter()
                .any(|known| role.eq_ignore_ascii_case(known)) =>
        {
            "text_entry"
        }
        _ => "content",
    }
}

fn historical_role(node: &Value) -> &'static str {
    match node
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "axheading" => "heading",
        "axstatictext" | "axtext" => "text",
        "axlist" => "list",
        "axlistitem" => "list_item",
        "axtable" | "axoutline" => "table",
        "axrow" => "row",
        "axcell" | "axcolumn" => "cell",
        "axlink" => "link",
        "axbutton" | "axcheckbox" | "axradiobutton" | "axtextfield" | "axtextarea"
        | "axcombobox" | "axpopupbutton" | "axslider" | "axswitch" => "control",
        "axdialog" | "axsheet" => "dialog",
        "axgroup" | "axsection" | "axlandmark" | "axwebarea" | "axwindow" => "region",
        "axparagraph" => "paragraph",
        "aximage" => "image",
        _ => "unknown",
    }
}

fn add_purpose(node: &mut Value) -> Result<(), String> {
    if node.get("purpose").is_some() {
        return Err("legacy node contains current semantics".into());
    }
    let purpose = historical_purpose(node);
    node.as_object_mut()
        .ok_or("node must be an object")?
        .insert("purpose".into(), json!(purpose));
    Ok(())
}

fn migrate_document(value: &mut Value) -> Result<(), String> {
    if !version(value, 2, 3)? {
        return Ok(());
    }
    if value.get("source_api").is_some() {
        return Err("legacy document contains current provenance".into());
    }
    for node in value
        .get_mut("nodes")
        .and_then(Value::as_object_mut)
        .ok_or("document nodes must be an object")?
        .values_mut()
    {
        add_purpose(node)?;
    }
    value["source_api"] = json!("macos_ax");
    value["schema_version"] = json!(3);
    Ok(())
}

fn migrate_projection(value: &mut Value, omissions: &Value) -> Result<(), String> {
    if value.get("source_api").is_some() {
        return Err("legacy projection contains current provenance".into());
    }
    let nodes = value
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or("projection nodes must be an array")?;
    // Input v3 deliberately omitted AXWindow roles and root chrome. Its
    // document-wide omission proves a window root existed, but cannot identify
    // one among multiple parentless regions (including children with omitted
    // parents). Never reinterpret every parentless region as a window root.
    let candidates: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.get("kind").and_then(Value::as_str) == Some("region")
                && ["parent_id", "role", "title", "value", "description"]
                    .iter()
                    .all(|field| node.get(field).is_none_or(Value::is_null))
        })
        .map(|(index, _)| index)
        .collect();
    let omitted_root_chrome = omissions.as_array().is_some_and(|omissions| {
        omissions.iter().any(|omission| {
            omission.get("reason").and_then(Value::as_str) == Some("application_chrome")
                && omission.get("omitted_node_count").and_then(Value::as_u64) == Some(0)
                && ["first_order", "last_order"]
                    .iter()
                    .all(|field| omission.get(field).is_none_or(Value::is_null))
                && omission.get("detail").and_then(Value::as_str)
                    == Some("Root AXWindow text is represented by source metadata instead of document content.")
        })
    });
    let explicit_window_roots = nodes
        .iter()
        .filter(|node| {
            node.get("role").and_then(Value::as_str) == Some("AXWindow")
                && node.get("parent_id").is_none_or(Value::is_null)
        })
        .count();
    let window_root = match (
        candidates.as_slice(),
        omitted_root_chrome,
        explicit_window_roots,
    ) {
        ([index], true, 0) => Some(*index),
        ([], false, _) | ([], true, 1..) => None,
        _ => return Err("legacy projection has ambiguous window-root purpose".into()),
    };
    for (index, node) in nodes.iter_mut().enumerate() {
        add_purpose(node)?;
        if Some(index) == window_root {
            node["purpose"] = json!("window_root");
        }
    }
    value["source_api"] = json!("macos_ax");
    Ok(())
}

#[derive(Deserialize)]
#[serde(remote = "LensDocument")]
struct DocumentWire {
    source_api: SourceApi,
    schema_version: u32,
    source: LensSource,
    roots: Vec<String>,
    nodes: BTreeMap<String, LensNode>,
    quality: ExtractionQuality,
    #[serde(default)]
    omitted_resource_ref_count: usize,
    diagnostics: Vec<String>,
}

impl<'de> Deserialize<'de> for LensDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        migrate_document(&mut value).map_err(D::Error::custom)?;
        DocumentWire::deserialize(value).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(remote = "LensContext")]
struct ContextWire {
    schema_version: u32,
    context_id: Uuid,
    revision: u64,
    sources: Vec<LensAccessibilitySource>,
    media: Vec<LensMediaAttachment>,
    media_omissions: Vec<LensMediaOmission>,
    quality: ExtractionQuality,
    diagnostics: Vec<String>,
}

impl<'de> Deserialize<'de> for LensContext {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        let legacy = version(&value, 4, 5).map_err(D::Error::custom)?;
        for source in value
            .get_mut("sources")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| D::Error::custom("context sources must be an array"))?
        {
            if let Some(document) = source.get_mut("document").filter(|v| !v.is_null()) {
                if document.get("schema_version").and_then(Value::as_u64)
                    != Some(if legacy { 2 } else { 3 })
                {
                    return Err(D::Error::custom(
                        "context and document schema versions do not match",
                    ));
                }
                if legacy {
                    migrate_document(document).map_err(D::Error::custom)?;
                }
            }
            if legacy {
                let capture = source
                    .get_mut("capture")
                    .ok_or_else(|| D::Error::custom("capture missing"))?;
                if !capture.is_object() {
                    return Err(D::Error::custom("capture must be an object"));
                }
                if capture.get("source_api").is_some() {
                    return Err(D::Error::custom(
                        "legacy capture contains current provenance",
                    ));
                }
                if capture.get("nodes").is_none() {
                    capture["nodes"] = json!([]);
                }
                for node in capture
                    .get_mut("nodes")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| D::Error::custom("capture nodes missing"))?
                {
                    if node.get("semantic_role").is_some() {
                        return Err(D::Error::custom("legacy capture contains current role"));
                    }
                    let role = historical_role(node);
                    add_purpose(node).map_err(D::Error::custom)?;
                    node["semantic_role"] = json!(role);
                }
                capture["source_api"] = json!("macos_ax");
            }
        }
        value["schema_version"] = json!(5);
        ContextWire::deserialize(value).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(remote = "LensInput")]
struct InputWire {
    schema_version: u32,
    context_id: Uuid,
    context_revision: u64,
    sources: Vec<LensInputSource>,
    media: Vec<LensMediaAttachment>,
    media_omissions: Vec<LensMediaOmission>,
    quality: ExtractionQuality,
}

impl<'de> Deserialize<'de> for LensInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        if version(&value, 3, 4).map_err(D::Error::custom)? {
            for source in value
                .get_mut("sources")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| D::Error::custom("input sources must be an array"))?
            {
                let omissions = source.get("omissions").cloned().unwrap_or(Value::Null);
                if let Some(document) = source.get_mut("document").filter(|v| !v.is_null()) {
                    migrate_projection(document, &omissions).map_err(D::Error::custom)?;
                }
            }
            value["schema_version"] = json!(4);
        }
        InputWire::deserialize(value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Value {
        json!({"application":"Fixture","window_title":"Window","bundle_id":"test.fixture","window_id":7})
    }
    fn document() -> Value {
        json!({"schema_version":2,"source":source(),"roots":["root"],"nodes":{"root":{"id":"root","order":0,"depth":0,"kind":"region","role":"AXWindow","title":"Window"}},"quality":"full","diagnostics":[]})
    }
    fn context() -> Value {
        json!({"schema_version":4,"context_id":Uuid::from_u128(1),"revision":1,"sources":[{"source_id":"s","target_id":"t","revision":1,"source":source(),"capture":{"quality":"full","nodes":[{"id":"root","order":0,"depth":0,"role":"AXWindow","subrole":"UnknownNativeSubrole"}]},"document":document(),"quality":"full"}],"media":[],"media_omissions":[],"quality":"full","diagnostics":[]})
    }

    #[test]
    fn legacy_document_and_context_migrate_once_with_raw_facts_unchanged() {
        let value: LensDocument = serde_json::from_value(document()).unwrap();
        assert_eq!(value.schema_version, 3);
        assert_eq!(value.source_api, SourceApi::MacosAx);
        assert_eq!(value.nodes["root"].purpose, NodePurpose::WindowRoot);
        let value: LensContext = serde_json::from_value(context()).unwrap();
        assert_eq!(value.schema_version, 5);
        assert_eq!(value.sources[0].capture.source_api, SourceApi::MacosAx);
        let node = &value.sources[0].capture.nodes[0];
        assert_eq!(node.semantic_role, SemanticRole::Region);
        assert_eq!(node.role.as_deref(), Some("AXWindow"));
        assert_eq!(node.subrole.as_deref(), Some("UnknownNativeSubrole"));
        let current = serde_json::to_value(&value).unwrap();
        assert_eq!(
            serde_json::from_value::<LensContext>(current).unwrap(),
            value
        );
    }

    #[test]
    fn legacy_input_projection_has_explicit_provenance_and_purpose() {
        let value = json!({"schema_version":3,"context_id":Uuid::from_u128(1),"context_revision":1,"sources":[{"source_id":"s","target_id":"t","source_revision":1,"source":source(),"document":{"nodes":[{"id":"text","kind":"control","role":"AXTextArea","value":"Editor"},{"id":"unknown","kind":"unknown","role":"CustomAXImageLike"}]},"quality":"full","omissions":[]}],"media":[],"media_omissions":[],"quality":"full"});
        let input: LensInput = serde_json::from_value(value).unwrap();
        assert_eq!(input.schema_version, 4);
        let doc = input.sources[0].document.as_ref().unwrap();
        assert_eq!(doc.source_api, SourceApi::MacosAx);
        assert_eq!(doc.nodes[0].purpose, NodePurpose::TextEntry);
        assert_eq!(doc.nodes[1].kind, SemanticRole::Unknown);
        assert_eq!(doc.nodes[1].role.as_deref(), Some("CustomAXImageLike"));
        assert_eq!(
            serde_json::from_value::<LensInput>(serde_json::to_value(&input).unwrap()).unwrap(),
            input
        );
    }

    fn projected_source(nodes: Value) -> Value {
        json!({"schema_version":3,"context_id":Uuid::from_u128(1),"context_revision":1,"sources":[{"source_id":"s","target_id":"t","source_revision":1,"source":source(),"document":{"nodes":nodes},"quality":"full","omissions":[]}],"media":[],"media_omissions":[],"quality":"full"})
    }

    fn root_chrome_omission() -> Value {
        json!([{"reason":"application_chrome","omitted_node_count":0,"detail":"Root AXWindow text is represented by source metadata instead of document content."}])
    }

    #[test]
    fn legacy_projection_recovers_only_an_unambiguous_omitted_window_root() {
        let mut old = projected_source(json!([
            {"id":"root","kind":"region"},
            {"id":"group","kind":"region","parent_id":"root"},
            {"id":"text","kind":"text","parent_id":"group","value":"Body"},
            {"id":"other","kind":"region","title":"Visible group"}
        ]));
        old["sources"][0]["omissions"] = root_chrome_omission();
        let migrated: LensInput = serde_json::from_value(old).unwrap();
        let document = migrated.sources[0].document.as_ref().unwrap();
        assert_eq!(document.nodes[0].purpose, NodePurpose::WindowRoot);
        assert_eq!(document.nodes[0].role, None);
        for node in &document.nodes[1..] {
            assert_eq!(node.purpose, NodePurpose::Content);
        }
        assert_eq!(
            serde_json::from_value::<LensInput>(serde_json::to_value(&migrated).unwrap()).unwrap(),
            migrated
        );
    }

    #[test]
    fn legacy_projection_rejects_unrecoverable_or_unrelated_root_evidence() {
        let root = json!({"id":"root","kind":"region"});
        let mut old = projected_source(json!([root.clone()]));
        // A blank AXWindow, AXGroup and AXWebArea all produced this same v3
        // node. No raw role or source-level omission means no recoverable fact.
        assert!(serde_json::from_value::<LensInput>(old.clone()).is_err());
        for omissions in [
            json!([{"reason":"token_budget","omitted_node_count":1}]),
            json!([{"reason":"application_chrome","omitted_node_count":0,"detail":"Unrelated omission"}]),
        ] {
            old["sources"][0]["omissions"] = omissions;
            assert!(serde_json::from_value::<LensInput>(old.clone()).is_err());
        }
        old["sources"][0]["omissions"] = root_chrome_omission();
        // Also covers a region whose parent was excluded by projection limits:
        // v3 removed that parent_id, so it cannot identify the window root.
        old["sources"][0]["document"]["nodes"] =
            json!([root, {"id":"orphaned-group","kind":"region"}]);
        assert!(serde_json::from_value::<LensInput>(old.clone()).is_err());
        old["sources"][0]["document"]["nodes"] =
            json!([{"id":"unrelated","kind":"region","title":"Visible group"}]);
        assert!(serde_json::from_value::<LensInput>(old).is_err());
    }

    #[test]
    fn legacy_projection_does_not_promote_other_parentless_nodes() {
        let old = projected_source(json!([
            {"id":"group","kind":"region","title":"Visible group"},
            {"id":"explicit-group","kind":"region","role":"AXGroup"},
            {"id":"web","kind":"region","role":"AXWebArea"},
            {"id":"text","kind":"text","value":"Body"}
        ]));
        let migrated: LensInput = serde_json::from_value(old).unwrap();
        for node in &migrated.sources[0].document.as_ref().unwrap().nodes {
            assert_eq!(node.purpose, NodePurpose::Content);
        }
    }

    #[test]
    fn explicit_legacy_window_role_does_not_attribute_its_omission_to_another_root() {
        let mut old = projected_source(json!([
            {"id":"explicit-window","kind":"unknown","role":"AXWindow"}
        ]));
        old["sources"][0]["omissions"] = root_chrome_omission();
        let migrated: LensInput = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(
            migrated.sources[0].document.as_ref().unwrap().nodes[0].purpose,
            NodePurpose::WindowRoot
        );
        old["sources"][0]["document"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"other-group","kind":"region"}));
        assert!(serde_json::from_value::<LensInput>(old).is_err());
    }

    #[test]
    fn projection_recovery_never_relaxes_current_or_mixed_schema_validation() {
        let mut old = projected_source(json!([{"id":"root","kind":"region"}]));
        old["sources"][0]["omissions"] = root_chrome_omission();
        for version in [0, 1, 2, 4, 5, u64::MAX] {
            let mut invalid = old.clone();
            invalid["schema_version"] = json!(version);
            assert!(serde_json::from_value::<LensInput>(invalid).is_err());
        }
        for (field, value) in [
            ("source_api", json!("macos_ax")),
            ("source_api", json!("future_native_api")),
        ] {
            let mut invalid = old.clone();
            invalid["sources"][0]["document"][field] = value;
            assert!(serde_json::from_value::<LensInput>(invalid).is_err());
        }
        old["sources"][0]["document"]["nodes"][0]["purpose"] = json!("window_root");
        assert!(serde_json::from_value::<LensInput>(old).is_err());
    }

    #[test]
    fn legacy_unavailable_capture_retains_original_omitted_nodes_default_only() {
        let mut old = context();
        old["sources"][0]["capture"] = json!({"quality":"unavailable"});
        old["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("document");
        let migrated: LensContext = serde_json::from_value(old.clone()).unwrap();
        assert!(migrated.sources[0].capture.nodes.is_empty());
        for invalid in [
            Value::Null,
            json!(false),
            json!(42),
            json!("capture"),
            json!([]),
        ] {
            let mut invalid_context = old.clone();
            invalid_context["sources"][0]["capture"] = invalid;
            assert!(serde_json::from_value::<LensContext>(invalid_context).is_err());
        }
        for invalid in [Value::Null, json!({}), json!("nodes")] {
            old["sources"][0]["capture"]["nodes"] = invalid;
            assert!(serde_json::from_value::<LensContext>(old.clone()).is_err());
        }
    }

    #[test]
    fn unknown_versions_and_mixed_generations_are_rejected() {
        for v in [0, 1, 3, 6, u64::MAX] {
            let mut invalid = context();
            invalid["schema_version"] = json!(v);
            assert!(serde_json::from_value::<LensContext>(invalid).is_err());
        }
        for (parent, child) in [(4, 3), (5, 2)] {
            let mut invalid = context();
            invalid["schema_version"] = json!(parent);
            invalid["sources"][0]["document"]["schema_version"] = json!(child);
            assert!(serde_json::from_value::<LensContext>(invalid).is_err());
        }
        let mut invalid = context();
        invalid["sources"][0]["capture"]["source_api"] = json!("macos_ax");
        assert!(serde_json::from_value::<LensContext>(invalid).is_err());
        for v in [0, 1, 4, 99] {
            let mut d = document();
            d["schema_version"] = json!(v);
            assert!(serde_json::from_value::<LensDocument>(d).is_err());
        }
    }

    #[test]
    fn current_missing_facts_never_infer_ax_and_future_api_values_reject() {
        let current =
            serde_json::to_value(serde_json::from_value::<LensContext>(context()).unwrap())
                .unwrap();
        for pointer in [
            "/sources/0/capture/source_api",
            "/sources/0/capture/nodes/0/semantic_role",
            "/sources/0/capture/nodes/0/purpose",
            "/sources/0/document/source_api",
            "/sources/0/document/nodes/root/purpose",
        ] {
            let mut invalid = current.clone();
            *invalid.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(
                serde_json::from_value::<LensContext>(invalid).is_err(),
                "{pointer}"
            );
            let mut missing = current.clone();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            missing
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                serde_json::from_value::<LensContext>(missing).is_err(),
                "missing {pointer}"
            );
        }
        let mut unknown = current.clone();
        unknown["sources"][0]["capture"]["source_api"] = json!("unknown");
        assert_eq!(
            serde_json::from_value::<LensContext>(unknown.clone())
                .unwrap()
                .sources[0]
                .capture
                .source_api,
            SourceApi::Unknown
        );
        unknown["sources"][0]["capture"]["source_api"] = json!("future_native_api");
        assert!(serde_json::from_value::<LensContext>(unknown).is_err());
        assert!(
            serde_json::from_value::<ExtractionResult>(json!({"quality":"unavailable"})).is_err()
        );
    }
}
