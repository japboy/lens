//! Access inspection is independent of extraction quality and selected-source binding.
use crate::PlatformError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceCapability {
    TargetSelection,
    StructuredExtraction,
    ImageCapture,
    Observation,
}

impl SourceCapability {
    pub const ALL: [Self; 4] = [
        Self::TargetSelection,
        Self::StructuredExtraction,
        Self::ImageCapture,
        Self::Observation,
    ];
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    Available,
    PermissionRequired,
    Restricted,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessAction {
    RequestPermission,
}

/// Application-level access. Available never proves selected-target identity,
/// continuity, content quality or the success of a future native call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "InspectionWire")]
pub struct CapabilityInspection {
    pub capability: SourceCapability,
    pub status: AccessStatus,
    pub request_action: Option<AccessAction>,
    pub diagnostics: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectionWire {
    capability: SourceCapability,
    status: AccessStatus,
    #[serde(deserialize_with = "present_request_action")]
    request_action: Option<AccessAction>,
    diagnostics: Vec<String>,
}

fn present_request_action<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<AccessAction>, D::Error> {
    Option::deserialize(deserializer)
}

impl TryFrom<InspectionWire> for CapabilityInspection {
    type Error = &'static str;
    fn try_from(wire: InspectionWire) -> Result<Self, Self::Error> {
        if wire.request_action.is_some() && wire.status != AccessStatus::PermissionRequired {
            return Err("only permission_required may offer a permission request");
        }
        Ok(Self {
            capability: wire.capability,
            status: wire.status,
            request_action: wire.request_action,
            diagnostics: wire.diagnostics,
        })
    }
}

pub trait CapabilityAccess: Send + Sync {
    /// Must not prompt, enumerate targets or start acquisition.
    fn inspect(&self, capability: SourceCapability) -> CapabilityInspection;
    /// Ok means the native action ran, not that permission was granted.
    fn request(&self, capability: SourceCapability) -> Result<(), PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_status_and_requestability_are_validated_at_ingress() {
        assert!(serde_json::from_value::<CapabilityInspection>(serde_json::json!({"capability":"structured_extraction","status":"available","diagnostics":[]})).is_err());
        for status in [
            "available",
            "permission_required",
            "restricted",
            "unsupported",
            "failed",
        ] {
            let mut value = serde_json::json!({"capability":"structured_extraction","status":status,"request_action":null,"diagnostics":[]});
            assert!(serde_json::from_value::<CapabilityInspection>(value.clone()).is_ok());
            value["request_action"] = serde_json::json!("request_permission");
            assert_eq!(
                serde_json::from_value::<CapabilityInspection>(value).is_ok(),
                status == "permission_required"
            );
        }
        for (field, value) in [
            ("status", "future"),
            ("capability", "future"),
            ("request_action", "future"),
        ] {
            let mut wire = serde_json::json!({"capability":"image_capture","status":"available","request_action":null,"diagnostics":[]});
            wire[field] = serde_json::json!(value);
            assert!(serde_json::from_value::<CapabilityInspection>(wire).is_err());
        }
    }
}
