//! Shared Desktop checks before the initial context and media become visible.

use port_platform::authority::TargetReadKey;
use std::collections::BTreeMap;
use usecase::{
    lens::LensMediaPayload,
    model::{LensStage, LensState},
    read_admission::{OperationAdmission, ReadAdmission},
    state::{validate_context_state, validated_payload_map},
};
use uuid::Uuid;

pub(crate) fn validate_result(operation_id: Uuid, next: &LensState) -> Result<(), String> {
    if next.input.is_some() {
        return validate_context_state(operation_id, next);
    }
    if next.operation_id != Some(operation_id)
        || next.stage != LensStage::Failed
        || next
            .context
            .as_ref()
            .is_none_or(|context| context.revision == 0)
        || next
            .target_set
            .as_ref()
            .is_none_or(|targets| targets.selection_id != operation_id)
        || next.projection.is_some()
    {
        return Err("failed source result has inconsistent context identity".into());
    }
    Ok(())
}

pub(crate) fn is_pending(current: &LensState, operation_id: Uuid) -> bool {
    current.operation_id == Some(operation_id)
        && current.context.is_none()
        && current.stage == LensStage::Extracting
}

pub(crate) fn allows_reads(
    admission: &ReadAdmission,
    next: &LensState,
    reads: &[TargetReadKey],
    registered: bool,
) -> bool {
    next.operation_id == Some(admission.operation_id())
        && admission.operation_state() == OperationAdmission::Active
        && if registered {
            reads.len() == next.target_set.as_ref().map_or(0, |set| set.targets.len())
                && admission.allows_all(reads)
        } else {
            reads.is_empty()
        }
}

pub(crate) fn payloads(
    payloads: Vec<LensMediaPayload>,
) -> Result<BTreeMap<String, LensMediaPayload>, String> {
    validated_payload_map(payloads)
}
