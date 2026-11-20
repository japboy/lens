//! Explicit actions are admitted from a fresh inspection, not an extraction failure.
pub use port_platform::trust::{
    AccessAction, AccessStatus, CapabilityInspection, SourceCapability,
};
use port_platform::{trust::CapabilityAccess, PlatformError};

pub fn inspect(access: &dyn CapabilityAccess) -> Vec<CapabilityInspection> {
    SourceCapability::ALL
        .into_iter()
        .map(|capability| access.inspect(capability))
        .collect()
}

pub fn request(
    access: &dyn CapabilityAccess,
    capability: SourceCapability,
) -> Result<CapabilityInspection, PlatformError> {
    let inspection = access.inspect(capability);
    if inspection.capability != capability
        || inspection.status != AccessStatus::PermissionRequired
        || inspection.request_action != Some(AccessAction::RequestPermission)
    {
        return Err(PlatformError::Operation(
            "This capability has no supported permission request action".into(),
        ));
    }
    access.request(capability)?;
    Ok(access.inspect(capability))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fake {
        status: AccessStatus,
        requests: AtomicUsize,
    }
    impl CapabilityAccess for Fake {
        fn inspect(&self, capability: SourceCapability) -> CapabilityInspection {
            CapabilityInspection {
                capability,
                status: self.status,
                request_action: (self.status == AccessStatus::PermissionRequired)
                    .then_some(AccessAction::RequestPermission),
                diagnostics: vec![],
            }
        }
        fn request(&self, _: SourceCapability) -> Result<(), PlatformError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[test]
    fn inspection_never_requests_and_request_reinspects_without_inventing_a_grant() {
        let fake = Fake {
            status: AccessStatus::PermissionRequired,
            requests: AtomicUsize::new(0),
        };
        assert_eq!(inspect(&fake).len(), 4);
        assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            request(&fake, SourceCapability::StructuredExtraction)
                .unwrap()
                .status,
            AccessStatus::PermissionRequired
        );
        assert_eq!(fake.requests.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn unavailable_or_already_available_capabilities_never_request_permission() {
        for status in [
            AccessStatus::Available,
            AccessStatus::Restricted,
            AccessStatus::Unsupported,
            AccessStatus::Failed,
        ] {
            let fake = Fake {
                status,
                requests: AtomicUsize::new(0),
            };
            assert!(request(&fake, SourceCapability::ImageCapture).is_err());
            assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
        }
    }
}
