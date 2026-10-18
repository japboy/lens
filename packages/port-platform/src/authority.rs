//! Portable correlation values for operation-scoped target acquisition.
//!
//! A receipt is not a native window identity or a permission grant. Adapters must
//! resolve it only through the issuing operation's live registry. Matching a read
//! key alone proves neither native target lifetime nor atomic tree/image capture.
//!
//! These primitives do not yet replace the production selection/read contracts.
//! Their usecase owner must issue monotonically increasing sequences and close
//! admission permanently on exhaustion, Stop, or target invalidation.

use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use uuid::Uuid;

/// An opaque, non-nil receipt, independent of native addresses and display facts.
///
/// The adapter issues a fresh UUID; this portable value only validates it.
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

/// Registry lookup correlation; possession alone never authorizes native access.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct TargetAuthority {
    pub operation_id: Uuid,
    pub receipt: TargetReceipt,
}

/// A nonzero u64 serialized only as a canonical decimal string, never a JSON number.
///
/// String encoding preserves all 64 bits across a JavaScript/WebView boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct ReadSequence(NonZeroU64);

impl ReadSequence {
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Returns None at u64::MAX; the owner must terminate, never reset or wrap.
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

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

/// Every axis must match before the lifecycle owner admits a read result.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct TargetReadKey {
    pub target: TargetAuthority,
    pub sequence: ReadSequence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn key() -> TargetReadKey {
        TargetReadKey {
            target: TargetAuthority {
                operation_id: Uuid::from_u128(1),
                receipt: TargetReceipt::try_from(Uuid::from_u128(2)).unwrap(),
            },
            sequence: ReadSequence::FIRST,
        }
    }

    #[test]
    fn receipts_validate_externally_issued_non_nil_uuid_values() {
        for uuid in [Uuid::from_u128(1), Uuid::from_u128(u128::MAX)] {
            let receipt = TargetReceipt::try_from(uuid).unwrap();
            assert_eq!(Uuid::from(receipt), uuid);
        }
    }

    #[test]
    fn receipt_wire_form_is_an_opaque_uuid_string() {
        let receipt = key().target.receipt;
        let encoded = json!("00000000-0000-0000-0000-000000000002");
        assert_eq!(serde_json::to_value(receipt).unwrap(), encoded);
        assert_eq!(
            serde_json::from_value::<TargetReceipt>(encoded).unwrap(),
            receipt
        );
    }

    #[test]
    fn receipts_reject_nil_malformed_and_native_address_values() {
        assert!(TargetReceipt::try_from(Uuid::nil()).is_err());
        for value in [
            Value::Null,
            json!(17),
            json!(true),
            json!(""),
            json!("17"),
            json!("not-a-uuid"),
            json!("00000000-0000-0000-0000-000000000000"),
            json!({ "window_id": 17 }),
            json!([]),
        ] {
            assert!(
                serde_json::from_value::<TargetReceipt>(value.clone()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn sequence_roundtrips_the_full_u64_range_as_strings() {
        for value in [1, 2, (1_u64 << 53) + 1, u64::MAX] {
            let sequence = ReadSequence::try_from(value.to_string()).unwrap();
            assert_eq!(sequence.get(), value);
            let encoded = serde_json::to_value(sequence).unwrap();
            assert_eq!(encoded, json!(value.to_string()));
            assert_eq!(
                serde_json::from_value::<ReadSequence>(encoded).unwrap(),
                sequence
            );
        }
    }

    #[test]
    fn sequence_rejects_zero_overflow_and_noncanonical_strings() {
        for value in [
            "",
            "0",
            "00",
            "01",
            "+1",
            "-1",
            " 1",
            "1 ",
            "1\n",
            "1.0",
            "1e0",
            "１",
            "18446744073709551616",
        ] {
            assert!(
                ReadSequence::try_from(value.to_owned()).is_err(),
                "{value:?}"
            );
            assert!(
                serde_json::from_value::<ReadSequence>(json!(value)).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn sequence_rejects_non_string_wire_values() {
        for value in [
            Value::Null,
            json!(0),
            json!(1),
            json!(u64::MAX),
            json!(true),
            json!([]),
            json!({}),
        ] {
            assert!(
                serde_json::from_value::<ReadSequence>(value.clone()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn checked_sequence_advancement_never_wraps() {
        assert_eq!(ReadSequence::FIRST.get(), 1);
        assert_eq!(ReadSequence::FIRST.checked_next().unwrap().get(), 2);
        let penultimate = ReadSequence::try_from((u64::MAX - 1).to_string()).unwrap();
        let last = penultimate.checked_next().unwrap();
        assert_eq!(last.get(), u64::MAX);
        assert_eq!(last.checked_next(), None);
    }

    #[test]
    fn read_keys_require_all_fields_and_reject_unknown_protocol_fields() {
        let encoded = serde_json::to_value(key()).unwrap();
        assert_eq!(
            serde_json::from_value::<TargetReadKey>(encoded.clone()).unwrap(),
            key()
        );
        for field in ["target", "sequence"] {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<TargetReadKey>(missing).is_err(),
                "{field}"
            );
        }
        for field in ["operation_id", "receipt"] {
            let mut missing = encoded.clone();
            missing["target"].as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<TargetReadKey>(missing).is_err(),
                "{field}"
            );
        }
        let mut extra_outer = encoded.clone();
        extra_outer["window_id"] = json!(17);
        assert!(serde_json::from_value::<TargetReadKey>(extra_outer).is_err());
        let mut extra_target = encoded;
        extra_target["target"]["window_id"] = json!(17);
        assert!(serde_json::from_value::<TargetReadKey>(extra_target).is_err());
    }

    #[test]
    fn every_correlation_axis_participates_in_equality() {
        let original = key();
        let mut operation_changed = original;
        operation_changed.target.operation_id = Uuid::from_u128(3);
        let mut receipt_changed = original;
        receipt_changed.target.receipt = TargetReceipt::try_from(Uuid::from_u128(3)).unwrap();
        let mut sequence_changed = original;
        sequence_changed.sequence = original.sequence.checked_next().unwrap();
        for changed in [operation_changed, receipt_changed, sequence_changed] {
            assert_ne!(original, changed);
        }
    }

    #[test]
    fn portable_values_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TargetReceipt>();
        assert_send_sync::<TargetAuthority>();
        assert_send_sync::<ReadSequence>();
        assert_send_sync::<TargetReadKey>();
    }
}
