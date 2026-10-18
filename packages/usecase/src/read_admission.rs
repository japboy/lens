//! Ephemeral operation-owned read authority. Hosts serialize transitions with publication.
use port_platform::authority::{ReadSequence, TargetAuthority, TargetReadKey};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationTerminalReason {
    Completed,
    Cancelled,
    TimedOut,
    AuthorityUncertain,
    NativeFailure,
    ProtocolFailure,
    SequenceExhausted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationAdmission {
    Active,
    Terminal(OperationTerminalReason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    Accepted,
    RejectedStale,
    Cancelled,
    TimedOut,
    NativeFailure,
    ProtocolFailure,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadState {
    Pending,
    Reading,
    Terminal(ReadOutcome),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupOutcome {
    ConfirmedClosed,
    UnconfirmedTermination,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupState {
    NotStarted,
    Running,
    Finished(CleanupOutcome),
}

#[derive(Debug)]
struct Registration {
    target: TargetAuthority,
    current: Option<(TargetReadKey, u64, ReadState)>,
}

/// Not serializable or cloneable: history cannot restore authority or rewind issuance.
#[derive(Debug)]
pub struct ReadAdmission {
    operation_id: Uuid,
    operation: OperationAdmission,
    cleanup: CleanupState,
    next: Option<ReadSequence>,
    targets: BTreeMap<String, Registration>,
    retired: HashSet<TargetAuthority>,
}
impl ReadAdmission {
    pub fn new(operation_id: Uuid) -> Self {
        Self {
            operation_id,
            operation: OperationAdmission::Active,
            cleanup: CleanupState::NotStarted,
            next: Some(ReadSequence::FIRST),
            targets: BTreeMap::new(),
            retired: HashSet::new(),
        }
    }
    pub fn operation_state(&self) -> OperationAdmission {
        self.operation
    }
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    pub fn cleanup_state(&self) -> CleanupState {
        self.cleanup
    }
    pub fn register(&mut self, id: String, target: TargetAuthority) -> Result<(), String> {
        if self.operation != OperationAdmission::Active
            || target.operation_id != self.operation_id
            || id.is_empty()
            || self.targets.contains_key(&id)
            || self.retired.contains(&target)
            || self.targets.values().any(|entry| entry.target == target)
        {
            return Err("Target registration has no live operation authority".into());
        }
        self.targets.insert(
            id,
            Registration {
                target,
                current: None,
            },
        );
        Ok(())
    }
    pub fn target(&self, id: &str) -> Result<TargetAuthority, String> {
        if self.operation != OperationAdmission::Active {
            return Err("Operation admission is closed".into());
        }
        self.targets
            .get(id)
            .map(|entry| entry.target)
            .ok_or_else(|| "Target registration is unavailable".into())
    }
    pub fn remove(&mut self, id: &str) -> Result<TargetAuthority, String> {
        let target = self
            .targets
            .remove(id)
            .ok_or("Target registration is unavailable")?
            .target;
        self.retired.insert(target);
        Ok(target)
    }
    pub fn issue(&mut self, id: &str, generation: u64) -> Result<TargetReadKey, String> {
        let target = self.target(id)?;
        if generation == 0 {
            return Err("Read generation must be non-zero".into());
        }
        if self.targets[id]
            .current
            .is_some_and(|record| generation < record.1)
        {
            return Err("Read generation was superseded".into());
        }
        let Some(sequence) = self.next else {
            self.terminate(OperationTerminalReason::SequenceExhausted);
            return Err("Operation read sequence is exhausted".into());
        };
        self.next = sequence.checked_next();
        let key = TargetReadKey { target, sequence };
        self.targets
            .get_mut(id)
            .expect("validated registration")
            .current = Some((key, generation, ReadState::Pending));
        Ok(key)
    }
    fn record(&self, key: TargetReadKey) -> Option<&(TargetReadKey, u64, ReadState)> {
        self.targets
            .values()
            .filter_map(|entry| entry.current.as_ref())
            .find(|record| record.0 == key)
    }
    pub fn read_state(&self, key: TargetReadKey) -> Option<ReadState> {
        self.record(key).map(|record| record.2)
    }
    pub fn start(&mut self, key: TargetReadKey) -> bool {
        if self.operation != OperationAdmission::Active {
            return false;
        }
        let Some(record) = self
            .targets
            .values_mut()
            .filter_map(|entry| entry.current.as_mut())
            .find(|record| record.0 == key)
        else {
            return false;
        };
        if record.2 != ReadState::Pending {
            return false;
        }
        record.2 = ReadState::Reading;
        true
    }
    pub fn admits(&self, key: TargetReadKey) -> bool {
        self.operation == OperationAdmission::Active
            && self.read_state(key) == Some(ReadState::Reading)
    }
    /// Rejects the entire incoming completion without consuming any valid outstanding read.
    pub fn accept_all(&mut self, keys: &[TargetReadKey]) -> bool {
        self.finish_all(keys, ReadOutcome::Accepted)
    }
    pub fn finish_all(&mut self, keys: &[TargetReadKey], outcome: ReadOutcome) -> bool {
        if !self.allows_all(keys) {
            return false;
        }
        for entry in self.targets.values_mut() {
            if let Some(record) = entry.current.as_mut() {
                if keys.contains(&record.0) {
                    record.2 = ReadState::Terminal(outcome);
                }
            }
        }
        true
    }
    pub fn allows_all(&self, keys: &[TargetReadKey]) -> bool {
        !keys.is_empty()
            && keys.iter().all(|key| self.admits(*key))
            && keys.iter().collect::<HashSet<_>>().len() == keys.len()
    }
    pub fn terminate(&mut self, reason: OperationTerminalReason) {
        if self.operation != OperationAdmission::Active {
            return;
        }
        self.operation = OperationAdmission::Terminal(reason);
        let outcome = match reason {
            OperationTerminalReason::TimedOut => ReadOutcome::TimedOut,
            OperationTerminalReason::NativeFailure => ReadOutcome::NativeFailure,
            OperationTerminalReason::ProtocolFailure => ReadOutcome::ProtocolFailure,
            _ => ReadOutcome::Cancelled,
        };
        for entry in self.targets.values_mut() {
            if let Some(record) = entry.current.as_mut() {
                if !matches!(record.2, ReadState::Terminal(_)) {
                    record.2 = ReadState::Terminal(outcome);
                }
            }
        }
    }
    /// Pause revokes outstanding reads without ending observation or rewinding issuance.
    pub fn cancel_reads(&mut self) {
        for entry in self.targets.values_mut() {
            if let Some(record) = entry.current.as_mut() {
                if matches!(record.2, ReadState::Pending | ReadState::Reading) {
                    record.2 = ReadState::Terminal(ReadOutcome::Cancelled);
                }
            }
        }
    }
    pub fn begin_cleanup(&mut self) {
        if self.operation != OperationAdmission::Active && self.cleanup == CleanupState::NotStarted
        {
            self.cleanup = CleanupState::Running;
        }
    }
    pub fn finish_cleanup(&mut self, outcome: CleanupOutcome) {
        if self.cleanup == CleanupState::Running {
            self.cleanup = CleanupState::Finished(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn admission() -> ReadAdmission {
        let mut state = ReadAdmission::new(Uuid::from_u128(1));
        for (index, id) in ["a", "b"].into_iter().enumerate() {
            state
                .register(
                    id.into(),
                    TargetAuthority {
                        operation_id: Uuid::from_u128(1),
                        receipt: Uuid::from_u128(index as u128 + 2).try_into().unwrap(),
                    },
                )
                .unwrap();
        }
        state
    }
    #[test]
    fn issuance_is_operation_wide_but_currentness_is_target_local() {
        let mut state = admission();
        let a = state.issue("a", 1).unwrap();
        let b = state.issue("b", 1).unwrap();
        assert_eq!(a.sequence.get() + 1, b.sequence.get());
        assert!(state.start(a));
        assert!(state.start(b));
        assert!(state.admits(a));
        let next_a = state.issue("a", 2).unwrap();
        assert!(!state.admits(a));
        assert!(state.admits(b));
        assert!(state.start(next_a));
        assert!(state.accept_all(&[next_a, b]));
        assert_eq!(state.operation_state(), OperationAdmission::Active);
        assert!(!state.accept_all(&[next_a, b]));
    }
    #[test]
    fn bad_completions_do_not_consume_valid_reads() {
        let mut state = admission();
        let a = state.issue("a", 1).unwrap();
        state.start(a);
        let wrong = TargetReadKey {
            target: TargetAuthority {
                receipt: Uuid::from_u128(99).try_into().unwrap(),
                ..a.target
            },
            ..a
        };
        assert!(!state.accept_all(&[a, wrong]));
        assert!(!state.accept_all(&[a, a]));
        assert!(!state.accept_all(&[]));
        assert!(state.admits(a));
        assert!(state.accept_all(&[a]));
    }
    #[test]
    fn aborted_batches_finish_once_and_bad_messages_do_not_consume_valid_reads() {
        for outcome in [
            ReadOutcome::Cancelled,
            ReadOutcome::TimedOut,
            ReadOutcome::NativeFailure,
            ReadOutcome::ProtocolFailure,
        ] {
            let mut state = admission();
            let a = state.issue("a", 1).unwrap();
            let b = state.issue("b", 1).unwrap();
            assert!(state.start(a));
            assert!(state.start(b));
            let wrong = TargetReadKey {
                sequence: "99".to_owned().try_into().unwrap(),
                ..a
            };
            assert!(!state.finish_all(&[a, wrong], outcome));
            assert!(state.admits(a));
            assert!(state.admits(b));
            assert!(state.finish_all(&[a, b], outcome));
            assert_eq!(state.read_state(a), Some(ReadState::Terminal(outcome)));
            assert_eq!(state.read_state(b), Some(ReadState::Terminal(outcome)));
            assert!(!state.accept_all(&[a, b]));
            assert_eq!(state.operation_state(), OperationAdmission::Active);
            assert!(state.issue("a", 2).is_ok());
        }
    }
    #[test]
    fn stop_and_cleanup_failure_never_reopen_admission() {
        let mut state = admission();
        let a = state.issue("a", 1).unwrap();
        state.start(a);
        state.terminate(OperationTerminalReason::Cancelled);
        state.begin_cleanup();
        state.finish_cleanup(CleanupOutcome::UnconfirmedTermination);
        state.terminate(OperationTerminalReason::Completed);
        assert!(!state.admits(a));
        assert!(!state.accept_all(&[a]));
        assert!(state.issue("b", 2).is_err());
        assert_eq!(
            state.operation_state(),
            OperationAdmission::Terminal(OperationTerminalReason::Cancelled)
        );
        assert_eq!(
            state.cleanup_state(),
            CleanupState::Finished(CleanupOutcome::UnconfirmedTermination)
        );
    }
    #[test]
    fn exhaustion_is_checked_and_terminal() {
        let mut state = admission();
        state.next = Some(u64::MAX.to_string().try_into().unwrap());
        let last = state.issue("a", 1).unwrap();
        assert_eq!(last.sequence.get(), u64::MAX);
        assert!(state.start(last));
        assert!(state.issue("b", 1).is_err());
        assert!(!state.admits(last));
        assert_eq!(
            state.operation_state(),
            OperationAdmission::Terminal(OperationTerminalReason::SequenceExhausted)
        );
    }
    #[test]
    fn removed_registration_cannot_be_revived() {
        let mut state = admission();
        let a = state.issue("a", 1).unwrap();
        state.start(a);
        let target = state.remove("a").unwrap();
        assert!(!state.admits(a));
        assert!(state.register("a".into(), target).is_err());
        assert!(state
            .register(
                "a".into(),
                TargetAuthority {
                    receipt: Uuid::from_u128(100).try_into().unwrap(),
                    ..target
                }
            )
            .is_ok());
    }
    #[test]
    fn old_generations_and_foreign_registrations_leave_current_reads_intact() {
        let mut state = admission();
        let a = state.issue("a", 2).unwrap();
        assert!(state.start(a));
        assert!(!state.start(a));
        assert!(state.issue("a", 1).is_err());
        assert!(state.issue("a", 0).is_err());
        assert!(state
            .register(
                "foreign".into(),
                TargetAuthority {
                    operation_id: Uuid::from_u128(2),
                    receipt: Uuid::from_u128(101).try_into().unwrap()
                }
            )
            .is_err());
        assert!(state.admits(a));
        let next = state.issue("a", 2).unwrap();
        assert!(next.sequence > a.sequence);
        assert!(!state.admits(a));
        assert!(state.start(next));
        assert!(state.accept_all(&[next]));
    }
    #[test]
    fn pause_irreversibly_cancels_reads_without_rewinding_the_live_operation() {
        let mut state = admission();
        let a = state.issue("a", 1).unwrap();
        assert!(state.start(a));
        state.begin_cleanup();
        assert_eq!(state.cleanup_state(), CleanupState::NotStarted);
        state.cancel_reads();
        assert_eq!(state.operation_state(), OperationAdmission::Active);
        assert_eq!(
            state.read_state(a),
            Some(ReadState::Terminal(ReadOutcome::Cancelled))
        );
        assert!(!state.start(a));
        assert!(!state.admits(a));
        let resumed = state.issue("a", 1).unwrap();
        assert_eq!(resumed.sequence.get(), a.sequence.get() + 1);
        assert!(state.start(resumed));
        assert!(!state.accept_all(&[a]));
        assert!(state.accept_all(&[resumed]));
    }
}
