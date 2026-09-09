//! Per-operation read issuance, independent of publication revisions and native effects.
//!
//! The host retains one instance for the complete operation, including previews and
//! failed refreshes. Reserving consumes a sequence even if the caller is cancelled.
//! The host must separately check current operation/lifecycle before dispatch and
//! publication: a read key is correlation, not permission or an atomic snapshot.

use port_platform::authority::{ReadSequence, TargetAuthority, TargetReadKey};
use uuid::Uuid;

/// The host synchronizes the operation issuer and independently admits its current
/// lifecycle. The builder reserves after validating targets and before any await.
pub trait ReadIssuer: Send + Sync {
    fn reserve(&self, target: TargetAuthority) -> Result<TargetReadKey, String>;
}

#[derive(Debug, PartialEq, Eq)]
enum IssuanceState {
    Ready(ReadSequence),
    Exhausted,
    Closed,
}

/// Intentionally not Clone or serializable: cloning/reloading a counter could reuse
/// an issued sequence. The host owns synchronization, not this portable state machine.
#[derive(Debug)]
pub struct OperationReadState {
    operation_id: Uuid,
    state: IssuanceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAdmissionError {
    InvalidOperation,
    WrongOperation,
    Exhausted,
    Closed,
}

impl OperationReadState {
    pub fn new(operation_id: Uuid) -> Result<Self, ReadAdmissionError> {
        if operation_id.is_nil() {
            return Err(ReadAdmissionError::InvalidOperation);
        }
        Ok(Self {
            operation_id,
            state: IssuanceState::Ready(ReadSequence::FIRST),
        })
    }

    /// Advance before handing the key to the caller. There is deliberately no
    /// rollback, reset, or retry method that can recreate an earlier sequence.
    pub fn reserve(
        &mut self,
        target: TargetAuthority,
    ) -> Result<TargetReadKey, ReadAdmissionError> {
        if target.operation_id != self.operation_id {
            return Err(ReadAdmissionError::WrongOperation);
        }
        let sequence = match self.state {
            IssuanceState::Ready(sequence) => sequence,
            IssuanceState::Exhausted => return Err(ReadAdmissionError::Exhausted),
            IssuanceState::Closed => return Err(ReadAdmissionError::Closed),
        };
        self.state = match sequence.checked_next() {
            Some(next) => IssuanceState::Ready(next),
            None => IssuanceState::Exhausted,
        };
        Ok(TargetReadKey { target, sequence })
    }

    pub fn close(&mut self) {
        self.state = IssuanceState::Closed;
    }

    pub fn is_closed(&self) -> bool {
        self.state == IssuanceState::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use port_platform::authority::TargetReceipt;

    fn target(operation: u128, receipt: u128) -> TargetAuthority {
        TargetAuthority {
            operation_id: Uuid::from_u128(operation),
            receipt: TargetReceipt::try_from(Uuid::from_u128(receipt)).unwrap(),
        }
    }

    #[test]
    fn failed_or_cancelled_reads_and_different_targets_never_reuse_sequences() {
        let mut state = OperationReadState::new(Uuid::from_u128(1)).unwrap();
        let abandoned = state.reserve(target(1, 2)).unwrap();
        let preview = state.reserve(target(1, 3)).unwrap();
        let retry = state.reserve(target(1, 2)).unwrap();
        assert_eq!(abandoned.sequence, ReadSequence::FIRST);
        assert_eq!(preview.sequence, abandoned.sequence.checked_next().unwrap());
        assert_eq!(retry.sequence, preview.sequence.checked_next().unwrap());
        assert_eq!(retry.target, abandoned.target);
    }

    #[test]
    fn wrong_operation_does_not_consume_and_close_is_terminal() {
        assert!(matches!(
            OperationReadState::new(Uuid::nil()),
            Err(ReadAdmissionError::InvalidOperation)
        ));
        let mut state = OperationReadState::new(Uuid::from_u128(1)).unwrap();
        assert_eq!(
            state.reserve(target(2, 3)),
            Err(ReadAdmissionError::WrongOperation)
        );
        assert_eq!(
            state.reserve(target(1, 3)).unwrap().sequence,
            ReadSequence::FIRST
        );
        state.close();
        state.close();
        assert_eq!(state.reserve(target(1, 3)), Err(ReadAdmissionError::Closed));
    }

    #[test]
    fn last_sequence_is_issued_once_and_exhaustion_never_wraps() {
        let last = ReadSequence::try_from(u64::MAX.to_string()).unwrap();
        let mut state = OperationReadState {
            operation_id: Uuid::from_u128(1),
            state: IssuanceState::Ready(last),
        };
        assert_eq!(state.reserve(target(1, 2)).unwrap().sequence, last);
        for _ in 0..2 {
            assert_eq!(
                state.reserve(target(1, 2)),
                Err(ReadAdmissionError::Exhausted)
            );
        }
        state.close();
        assert_eq!(state.reserve(target(1, 2)), Err(ReadAdmissionError::Closed));
    }
}
