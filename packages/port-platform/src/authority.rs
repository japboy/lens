//! Correlation values for registered targets; possession alone never grants native access.
//!
//! Adapters issue receipts independently of native handles and resolve them only in
//! the issuing operation's live registry. Read sequences belong to the caller's
//! operation, not a provider timestamp or a claim of atomic acquisition.

use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct TargetReceipt(Uuid);

impl TryFrom<Uuid> for TargetReceipt {
    type Error = &'static str;

    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        if value.is_nil() {
            Err("target receipt must not be nil")
        } else {
            Ok(Self(value))
        }
    }
}

impl From<TargetReceipt> for Uuid {
    fn from(value: TargetReceipt) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct TargetAuthority {
    pub operation_id: Uuid,
    pub receipt: TargetReceipt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct ReadSequence(NonZeroU64);

// Decimal strings preserve all 64 bits across the JSON/WebView boundary. Numeric
// JSON values and noncanonical spellings are deliberately not compatibility inputs.
impl TryFrom<String> for ReadSequence {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parsed = value
            .parse::<NonZeroU64>()
            .map_err(|_| "read sequence must be a nonzero canonical decimal u64 string")?;
        if parsed.to_string() != value {
            return Err("read sequence must be a nonzero canonical decimal u64 string");
        }
        Ok(Self(parsed))
    }
}

impl From<ReadSequence> for String {
    fn from(value: ReadSequence) -> Self {
        value.0.to_string()
    }
}

impl ReadSequence {
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    /// Exhaustion is terminal for this operation; it must never wrap to an old read.
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct TargetReadKey {
    pub target: TargetAuthority,
    pub sequence: ReadSequence,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_reject_native_numbers_missing_values_and_nil() {
        for invalid in ["17", "null", "\"00000000-0000-0000-0000-000000000000\""] {
            assert!(serde_json::from_str::<TargetReceipt>(invalid).is_err());
        }
        let receipt = TargetReceipt::try_from(Uuid::from_u128(7)).unwrap();
        let encoded = serde_json::to_string(&receipt).unwrap();
        assert_eq!(
            serde_json::from_str::<TargetReceipt>(&encoded).unwrap(),
            receipt
        );
    }

    #[test]
    fn read_sequence_is_nonzero_and_never_wraps() {
        for invalid in [
            "0",
            "1",
            "\"0\"",
            "\"01\"",
            "\"+1\"",
            "\" 1\"",
            "\"18446744073709551616\"",
        ] {
            assert!(serde_json::from_str::<ReadSequence>(invalid).is_err());
        }
        assert_eq!(
            serde_json::to_string(&ReadSequence::FIRST).unwrap(),
            "\"1\""
        );
        assert_eq!(
            serde_json::to_string(&ReadSequence::FIRST.checked_next().unwrap()).unwrap(),
            "\"2\""
        );
        let last = ReadSequence::try_from(u64::MAX.to_string()).unwrap();
        let encoded = serde_json::to_string(&last).unwrap();
        assert_eq!(encoded, "\"18446744073709551615\"");
        assert_eq!(
            serde_json::from_str::<ReadSequence>(&encoded).unwrap(),
            last
        );
        assert_eq!(last.checked_next(), None);
    }

    #[test]
    fn correlation_requires_every_axis_and_rejects_extra_authority() {
        let key = TargetReadKey {
            target: TargetAuthority {
                operation_id: Uuid::from_u128(1),
                receipt: TargetReceipt::try_from(Uuid::from_u128(2)).unwrap(),
            },
            sequence: ReadSequence::FIRST,
        };
        let value = serde_json::to_value(key).unwrap();
        for field in ["target", "sequence"] {
            let mut invalid = value.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<TargetReadKey>(invalid).is_err());
        }
        let mut invalid = value.clone();
        invalid["target"]["window_id"] = serde_json::json!(17);
        assert!(serde_json::from_value::<TargetReadKey>(invalid).is_err());
        for changed in [
            TargetReadKey {
                sequence: key.sequence.checked_next().unwrap(),
                ..key
            },
            TargetReadKey {
                target: TargetAuthority {
                    operation_id: Uuid::from_u128(3),
                    ..key.target
                },
                ..key
            },
            TargetReadKey {
                target: TargetAuthority {
                    receipt: TargetReceipt::try_from(Uuid::from_u128(3)).unwrap(),
                    ..key.target
                },
                ..key
            },
        ] {
            assert_ne!(key, changed);
        }
    }
}
