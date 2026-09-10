use crate::{model::WindowIdentity, PlatformError};
use serde::{Deserialize, Serialize};
use std::{
    num::NonZeroU64,
    task::{Context, Poll},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowObservationNotification {
    WindowTitleChanged,
    WindowMoved,
    WindowResized,
    WindowDestroyed,
    ApplicationChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowObservationEvent {
    pub operation_id: Uuid,
    pub context_id: Uuid,
    pub source_registration_id: Uuid,
    pub observer_epoch: NonZeroU64,
    pub receipt: crate::authority::TargetReceipt,
    pub notification: WindowObservationNotification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowObservationStart {
    pub registered_notifications: Vec<WindowObservationNotification>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ObservationRequest {
    pub operation_id: Uuid,
    pub context_id: Uuid,
    pub source_registration_id: Uuid,
    pub observer_epoch: NonZeroU64,
    pub identity: WindowIdentity,
}

/// Native callbacks/resources stay inside the implementation. Explicit close reports
/// teardown failure; Drop must perform the implementation's best-effort cleanup.
pub trait ObservationRegistration: Send {
    fn close(&mut self) -> Result<(), PlatformError>;
}

/// Bounded, coalescing invalidations. Ready(None) is terminal, Pending is not closure.
pub trait ObservationEvents: Send {
    fn poll_next(&mut self, context: &mut Context<'_>) -> Poll<Option<WindowObservationEvent>>;
}

pub type WindowObservationRegistration = Box<dyn ObservationRegistration>;
pub type WindowObservationReceiver = Box<dyn ObservationEvents>;

pub struct ObservationSession {
    pub registration: WindowObservationRegistration,
    pub events: WindowObservationReceiver,
    pub start: WindowObservationStart,
}

pub trait Observation: Send + Sync {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_epoch_and_unknown_notification_are_independently_rejected() {
        let valid = serde_json::json!({
            "operation_id":Uuid::from_u128(1),"context_id":Uuid::from_u128(2),
            "source_registration_id":Uuid::from_u128(3),"observer_epoch":1,"receipt":Uuid::from_u128(17),
            "notification":"window_destroyed"
        });
        let mut zero_epoch = valid.clone();
        zero_epoch["observer_epoch"] = serde_json::json!(0);
        let mut unknown_notification = valid.clone();
        unknown_notification["notification"] = serde_json::json!("unknown");
        for value in [zero_epoch, unknown_notification] {
            assert!(serde_json::from_value::<WindowObservationEvent>(value).is_err());
        }
        assert!(serde_json::from_value::<WindowObservationEvent>(valid).is_ok());
    }

    #[test]
    fn portable_service_objects_are_sendable_without_native_resource_types() {
        fn assert_send<T: Send>() {}
        assert_send::<ObservationSession>();
        assert_send::<Box<dyn crate::selection::TargetSelection>>();
        assert_send::<Box<dyn crate::accessibility::Accessibility>>();
        assert_send::<Box<dyn crate::capture::Capture>>();
        assert_send::<Box<dyn Observation>>();
        assert_send::<Box<dyn crate::trust::AccessibilityTrust>>();
    }
    #[test]
    fn observation_event_requires_closed_notification_and_nonzero_epoch() {
        let event: WindowObservationEvent = serde_json::from_str(
            r#"{
                "operation_id":"00000000-0000-0000-0000-000000000001",
                "context_id":"00000000-0000-0000-0000-000000000002",
                "source_registration_id":"00000000-0000-0000-0000-000000000003",
                "observer_epoch":4,
                "receipt":"00000000-0000-0000-0000-000000000011",
                "notification":"window_title_changed"
            }"#,
        )
        .expect("valid event");
        assert_eq!(event.observer_epoch.get(), 4);
        assert_eq!(
            event.notification,
            WindowObservationNotification::WindowTitleChanged
        );

        let invalid = serde_json::from_str::<WindowObservationEvent>(
            r#"{
                "operation_id":"00000000-0000-0000-0000-000000000001",
                "context_id":"00000000-0000-0000-0000-000000000002",
                "source_registration_id":"00000000-0000-0000-0000-000000000003",
                "observer_epoch":0,
                "receipt":"00000000-0000-0000-0000-000000000011",
                "notification":"unbounded_native_detail"
            }"#,
        );
        assert!(invalid.is_err());
    }
}
