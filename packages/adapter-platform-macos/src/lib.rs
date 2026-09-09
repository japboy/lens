//! macOS native effects. Domain conversion and application state do not belong here.

#[cfg(not(target_os = "macos"))]
compile_error!("adapter-platform-macos requires a macOS target");

mod capture;
mod extraction;
mod geometry;
pub mod presentation;
mod source;

#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;

fn validate_read(
    actual: Option<port_platform::authority::TargetReadKey>,
    expected: Option<port_platform::authority::TargetReadKey>,
) -> Result<(), port_platform::PlatformError> {
    if actual == expected {
        Ok(())
    } else {
        Err(port_platform::PlatformError::InvalidResponse(
            "native response read key does not match its request".into(),
        ))
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use port_platform::authority::{ReadSequence, TargetAuthority, TargetReadKey, TargetReceipt};
    use uuid::Uuid;

    #[test]
    fn registered_responses_require_both_operation_and_receipt_and_legacy_cannot_claim_them() {
        let target = TargetAuthority {
            operation_id: Uuid::from_u128(1),
            receipt: TargetReceipt::try_from(Uuid::from_u128(2)).unwrap(),
        };
        let expected = TargetReadKey {
            target,
            sequence: ReadSequence::FIRST,
        };
        assert!(validate_read(Some(expected), Some(expected)).is_ok());
        assert!(validate_read(None, None).is_ok());
        assert!(validate_read(None, Some(expected)).is_err());
        assert!(validate_read(Some(expected), None).is_err());
        assert!(validate_read(
            Some(TargetReadKey {
                sequence: expected.sequence.checked_next().unwrap(),
                ..expected
            }),
            Some(expected)
        )
        .is_err());
        for other in [
            TargetAuthority {
                operation_id: Uuid::from_u128(3),
                ..target
            },
            TargetAuthority {
                receipt: TargetReceipt::try_from(Uuid::from_u128(3)).unwrap(),
                ..target
            },
        ] {
            assert!(validate_read(
                Some(TargetReadKey {
                    target: other,
                    ..expected
                }),
                Some(expected)
            )
            .is_err());
        }
    }
}
