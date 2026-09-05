//! Application-owned finite observation, delivery and Agent authority; canonical projection is domain-owned.

pub use domain::projection::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    num::{NonZeroU64, NonZeroUsize},
};
use thiserror::Error;
use uuid::Uuid;

/// The projection known to have been applied by the current ACP session epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentProjectionCursor {
    None,
    Applied { projection: ProjectionRef },
}

/// Local authority attached to one serial ACP prompt and all of its updates/completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentTurnKey {
    operation_id: Uuid,
    context_id: Uuid,
    session_epoch: NonZeroU64,
    turn_id: Uuid,
    base_projection: AgentProjectionCursor,
    target_projection: ProjectionRef,
}

impl AgentTurnKey {
    pub fn try_new(
        operation_id: Uuid,
        context_id: Uuid,
        session_epoch: NonZeroU64,
        turn_id: Uuid,
        base_projection: AgentProjectionCursor,
        target_projection: ProjectionRef,
    ) -> Result<Self, AgentTurnKeyError> {
        if let AgentProjectionCursor::Applied { projection } = &base_projection {
            if projection.revision >= target_projection.revision {
                return Err(AgentTurnKeyError::NonAdvancingRevision);
            }
            if projection.digest == target_projection.digest {
                return Err(AgentTurnKeyError::UnchangedDigest);
            }
        }
        Ok(Self {
            operation_id,
            context_id,
            session_epoch,
            turn_id,
            base_projection,
            target_projection,
        })
    }

    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(&self) -> Uuid {
        self.context_id
    }

    pub fn session_epoch(&self) -> NonZeroU64 {
        self.session_epoch
    }

    pub fn turn_id(&self) -> Uuid {
        self.turn_id
    }

    pub fn base_projection(&self) -> &AgentProjectionCursor {
        &self.base_projection
    }

    pub fn target_projection(&self) -> &ProjectionRef {
        &self.target_projection
    }

    /// Classifies whether this completed turn may publish a candidate for the current state.
    pub fn candidate_authority(
        &self,
        completed_turn: &AgentTurnKey,
        operation_id: Uuid,
        context_id: Uuid,
        session_epoch: NonZeroU64,
        latest_projection: &ProjectionRef,
    ) -> CandidateAuthority {
        if self != completed_turn {
            CandidateAuthority::TurnKeyMismatch
        } else if self.operation_id != operation_id {
            CandidateAuthority::OperationMismatch
        } else if self.context_id != context_id {
            CandidateAuthority::ContextMismatch
        } else if self.session_epoch != session_epoch {
            CandidateAuthority::EpochMismatch
        } else if &self.target_projection != latest_projection {
            CandidateAuthority::ProjectionMismatch
        } else {
            CandidateAuthority::Authoritative
        }
    }

    /// Advances the conversation cursor after a definitive non-cancelled response.
    ///
    /// Candidate display validation is deliberately not an input: the Agent may have applied the
    /// projection even when its representation is not displayable.
    pub fn applied_cursor_after_definitive_response(&self) -> AgentProjectionCursor {
        AgentProjectionCursor::Applied {
            projection: self.target_projection.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum AgentTurnKeyError {
    #[error("an Agent delta turn must advance the projection revision")]
    NonAdvancingRevision,
    #[error("an Agent delta turn cannot advance revision while retaining the same digest")]
    UnchangedDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAuthority {
    Authoritative,
    TurnKeyMismatch,
    OperationMismatch,
    ContextMismatch,
    EpochMismatch,
    ProjectionMismatch,
}

/// User-controlled lifecycle for one fixed source observation registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationLifecycle {
    Watching,
    Paused,
    Stopped,
}

/// Capacity-one source-local invalidation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRefreshState {
    Clean,
    Coalescing,
    Refreshing,
    DirtyWhileRefreshing,
}

/// Immutable operation/context/source scope for one native registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceRegistrationAuthority {
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
}

impl SourceRegistrationAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid, source_registration_id: Uuid) -> Self {
        Self {
            operation_id,
            context_id,
            source_registration_id,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }

    pub fn source_registration_id(self) -> Uuid {
        self.source_registration_id
    }
}

/// Authority copied into one native registration callback context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceObserverToken {
    authority: SourceRegistrationAuthority,
    observer_epoch: NonZeroU64,
}

impl SourceObserverToken {
    pub fn authority(self) -> SourceRegistrationAuthority {
        self.authority
    }

    pub fn observer_epoch(self) -> NonZeroU64 {
        self.observer_epoch
    }
}

/// Every event that may change one source's observation coordination state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObservationEvent {
    Notification { observer: SourceObserverToken },
    CoalescingElapsed { token: SourceWorkToken },
    RefreshCompleted { token: SourceWorkToken },
    Pause,
    Resume,
    Stop,
}

/// Authority for one coalescing timer and the refresh it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceWorkToken {
    observer: SourceObserverToken,
    work_generation: NonZeroU64,
}

impl SourceWorkToken {
    pub fn observer(self) -> SourceObserverToken {
        self.observer
    }

    pub fn authority(self) -> SourceRegistrationAuthority {
        self.observer.authority
    }

    pub fn observer_epoch(self) -> NonZeroU64 {
        self.observer.observer_epoch
    }

    pub fn work_generation(self) -> NonZeroU64 {
        self.work_generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationRejectionReason {
    NotWatching {
        lifecycle: ObservationLifecycle,
    },
    EpochMismatch {
        expected: NonZeroU64,
        received: NonZeroU64,
    },
    RegistrationMismatch {
        expected: SourceRegistrationAuthority,
        received: SourceRegistrationAuthority,
    },
    WorkTokenMismatch {
        expected: Option<SourceWorkToken>,
        received: SourceWorkToken,
    },
    CoalescingStateMismatch {
        state: SourceRefreshState,
    },
    RefreshCompletionStateMismatch {
        state: SourceRefreshState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExhaustedCoordinationCounter {
    ObserverEpoch,
    WorkGeneration,
}

/// Declarative effect that the native/async integration layer must perform, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceObservationEffect {
    CoalescingStarted {
        token: SourceWorkToken,
    },
    InvalidationCoalesced {
        token: SourceWorkToken,
    },
    DirtyLatched {
        active: SourceWorkToken,
    },
    DirtyAlreadyLatched {
        active: SourceWorkToken,
    },
    RefreshStarted {
        token: SourceWorkToken,
    },
    RefreshSettled {
        token: SourceWorkToken,
    },
    FollowUpCoalescingStarted {
        token: SourceWorkToken,
    },
    Paused {
        observer: SourceObserverToken,
    },
    AlreadyPaused,
    ResumedWithFullRefresh {
        token: SourceWorkToken,
    },
    AlreadyWatching,
    Stopped,
    AlreadyStopped,
    StoppedAtGenerationLimit {
        counter: ExhaustedCoordinationCounter,
    },
    Rejected {
        reason: ObservationRejectionReason,
    },
}

/// Pure state machine for callback admission, capacity-one coalescing, and Pause/Resume epochs.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceObservationCoordinator {
    authority: SourceRegistrationAuthority,
    lifecycle: ObservationLifecycle,
    observer_epoch: NonZeroU64,
    last_work_generation: u64,
    active_work: Option<SourceWorkToken>,
    refresh: SourceRefreshState,
}

impl SourceObservationCoordinator {
    pub fn new(authority: SourceRegistrationAuthority, observer_epoch: NonZeroU64) -> Self {
        Self {
            authority,
            lifecycle: ObservationLifecycle::Watching,
            observer_epoch,
            last_work_generation: 0,
            active_work: None,
            refresh: SourceRefreshState::Clean,
        }
    }

    pub fn lifecycle(&self) -> ObservationLifecycle {
        self.lifecycle
    }

    pub fn observer_epoch(&self) -> NonZeroU64 {
        self.observer_epoch
    }

    pub fn observer_token(&self) -> SourceObserverToken {
        SourceObserverToken {
            authority: self.authority,
            observer_epoch: self.observer_epoch,
        }
    }

    pub fn refresh(&self) -> SourceRefreshState {
        self.refresh
    }

    pub fn active_work(&self) -> Option<SourceWorkToken> {
        self.active_work
    }

    pub fn apply(&mut self, event: SourceObservationEvent) -> SourceObservationEffect {
        match event {
            SourceObservationEvent::Notification { observer } => {
                if let Some(rejected) = self.reject_observer_event(observer) {
                    return rejected;
                }
                match self.refresh {
                    SourceRefreshState::Clean => {
                        let Some(token) = self.issue_work_token() else {
                            return self.fail_closed_at_generation_limit(
                                ExhaustedCoordinationCounter::WorkGeneration,
                            );
                        };
                        self.refresh = SourceRefreshState::Coalescing;
                        SourceObservationEffect::CoalescingStarted { token }
                    }
                    SourceRefreshState::Coalescing => {
                        SourceObservationEffect::InvalidationCoalesced {
                            token: self.active_work.expect("coalescing owns one work token"),
                        }
                    }
                    SourceRefreshState::Refreshing => {
                        self.refresh = SourceRefreshState::DirtyWhileRefreshing;
                        SourceObservationEffect::DirtyLatched {
                            active: self.active_work.expect("refresh owns one work token"),
                        }
                    }
                    SourceRefreshState::DirtyWhileRefreshing => {
                        SourceObservationEffect::DirtyAlreadyLatched {
                            active: self.active_work.expect("dirty refresh owns one work token"),
                        }
                    }
                }
            }
            SourceObservationEvent::CoalescingElapsed { token } => {
                if let Some(rejected) = self.reject_work_event(token) {
                    return rejected;
                }
                if self.refresh == SourceRefreshState::Coalescing {
                    self.refresh = SourceRefreshState::Refreshing;
                    SourceObservationEffect::RefreshStarted { token }
                } else {
                    SourceObservationEffect::Rejected {
                        reason: ObservationRejectionReason::CoalescingStateMismatch {
                            state: self.refresh,
                        },
                    }
                }
            }
            SourceObservationEvent::RefreshCompleted { token } => {
                if let Some(rejected) = self.reject_work_event(token) {
                    return rejected;
                }
                match self.refresh {
                    SourceRefreshState::Refreshing => {
                        self.refresh = SourceRefreshState::Clean;
                        self.active_work = None;
                        SourceObservationEffect::RefreshSettled { token }
                    }
                    SourceRefreshState::DirtyWhileRefreshing => {
                        let Some(next) = self.issue_work_token() else {
                            return self.fail_closed_at_generation_limit(
                                ExhaustedCoordinationCounter::WorkGeneration,
                            );
                        };
                        self.refresh = SourceRefreshState::Coalescing;
                        SourceObservationEffect::FollowUpCoalescingStarted { token: next }
                    }
                    SourceRefreshState::Clean | SourceRefreshState::Coalescing => {
                        SourceObservationEffect::Rejected {
                            reason: ObservationRejectionReason::RefreshCompletionStateMismatch {
                                state: self.refresh,
                            },
                        }
                    }
                }
            }
            SourceObservationEvent::Pause => self.pause(),
            SourceObservationEvent::Resume => self.resume(),
            SourceObservationEvent::Stop => self.stop(),
        }
    }

    fn reject_work_event(&self, received: SourceWorkToken) -> Option<SourceObservationEffect> {
        if let Some(rejected) = self.reject_observer_event(received.observer) {
            return Some(rejected);
        }
        if self.active_work != Some(received) {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::WorkTokenMismatch {
                    expected: self.active_work,
                    received,
                },
            })
        } else {
            None
        }
    }

    fn reject_observer_event(
        &self,
        received: SourceObserverToken,
    ) -> Option<SourceObservationEffect> {
        if self.lifecycle != ObservationLifecycle::Watching {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: self.lifecycle,
                },
            })
        } else if received.authority != self.authority {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::RegistrationMismatch {
                    expected: self.authority,
                    received: received.authority,
                },
            })
        } else if received.observer_epoch != self.observer_epoch {
            Some(SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::EpochMismatch {
                    expected: self.observer_epoch,
                    received: received.observer_epoch,
                },
            })
        } else {
            None
        }
    }

    fn pause(&mut self) -> SourceObservationEffect {
        match self.lifecycle {
            ObservationLifecycle::Watching => match self.advance_observer_epoch() {
                Some(_) => {
                    self.lifecycle = ObservationLifecycle::Paused;
                    self.refresh = SourceRefreshState::Clean;
                    self.active_work = None;
                    SourceObservationEffect::Paused {
                        observer: self.observer_token(),
                    }
                }
                None => self
                    .fail_closed_at_generation_limit(ExhaustedCoordinationCounter::ObserverEpoch),
            },
            ObservationLifecycle::Paused => SourceObservationEffect::AlreadyPaused,
            ObservationLifecycle::Stopped => SourceObservationEffect::AlreadyStopped,
        }
    }

    fn resume(&mut self) -> SourceObservationEffect {
        match self.lifecycle {
            ObservationLifecycle::Watching => SourceObservationEffect::AlreadyWatching,
            ObservationLifecycle::Paused => match self.advance_observer_epoch() {
                Some(_) => {
                    let Some(token) = self.issue_work_token() else {
                        return self.fail_closed_at_generation_limit(
                            ExhaustedCoordinationCounter::WorkGeneration,
                        );
                    };
                    self.lifecycle = ObservationLifecycle::Watching;
                    self.refresh = SourceRefreshState::Refreshing;
                    SourceObservationEffect::ResumedWithFullRefresh { token }
                }
                None => self
                    .fail_closed_at_generation_limit(ExhaustedCoordinationCounter::ObserverEpoch),
            },
            ObservationLifecycle::Stopped => SourceObservationEffect::AlreadyStopped,
        }
    }

    fn stop(&mut self) -> SourceObservationEffect {
        if self.lifecycle == ObservationLifecycle::Stopped {
            SourceObservationEffect::AlreadyStopped
        } else {
            self.lifecycle = ObservationLifecycle::Stopped;
            self.refresh = SourceRefreshState::Clean;
            self.active_work = None;
            SourceObservationEffect::Stopped
        }
    }

    fn advance_observer_epoch(&mut self) -> Option<NonZeroU64> {
        let next = self.observer_epoch.get().checked_add(1)?;
        self.observer_epoch = NonZeroU64::new(next).expect("a positive value plus one is non-zero");
        Some(self.observer_epoch)
    }

    fn issue_work_token(&mut self) -> Option<SourceWorkToken> {
        let next = self.last_work_generation.checked_add(1)?;
        let work_generation = NonZeroU64::new(next).expect("zero plus one is non-zero");
        self.last_work_generation = next;
        let token = SourceWorkToken {
            observer: self.observer_token(),
            work_generation,
        };
        self.active_work = Some(token);
        Some(token)
    }

    fn fail_closed_at_generation_limit(
        &mut self,
        counter: ExhaustedCoordinationCounter,
    ) -> SourceObservationEffect {
        self.lifecycle = ObservationLifecycle::Stopped;
        self.refresh = SourceRefreshState::Clean;
        self.active_work = None;
        SourceObservationEffect::StoppedAtGenerationLimit { counter }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineDeliveryState {
    Pending,
    InFlight { turn_id: Uuid },
    Acknowledged { session_epoch: NonZeroU64 },
}

/// Immutable operation/context scope shared by semantic events and an Agent timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SemanticContextAuthority {
    operation_id: Uuid,
    context_id: Uuid,
}

impl SemanticContextAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid) -> Self {
        Self {
            operation_id,
            context_id,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }
}

/// One ordered semantic change between two complete projection identities.
#[derive(Debug, PartialEq, Eq)]
pub struct SemanticTimelineEvent {
    authority: SemanticContextAuthority,
    event_id: Uuid,
    base_projection: ProjectionRef,
    target_projection: ProjectionRef,
    payload: serde_json::Value,
    canonical_payload_bytes: Box<[u8]>,
    delivery: TimelineDeliveryState,
}

impl SemanticTimelineEvent {
    pub fn try_new<T: Serialize>(
        authority: SemanticContextAuthority,
        event_id: Uuid,
        base_projection: ProjectionRef,
        target_projection: ProjectionRef,
        payload: &T,
    ) -> Result<Self, SemanticTimelineEventError> {
        if base_projection.revision >= target_projection.revision
            || base_projection.digest == target_projection.digest
        {
            return Err(SemanticTimelineEventError::InvalidProjectionTransition);
        }
        let canonical = CanonicalProjection::from_serializable(payload)
            .map_err(SemanticTimelineEventError::Canonicalization)?;
        let payload = serde_json::from_slice(canonical.bytes())
            .map_err(SemanticTimelineEventError::CanonicalPayloadDecode)?;
        Ok(Self {
            authority,
            event_id,
            base_projection,
            target_projection,
            payload,
            canonical_payload_bytes: canonical.bytes().to_vec().into_boxed_slice(),
            delivery: TimelineDeliveryState::Pending,
        })
    }

    pub fn event_id(&self) -> Uuid {
        self.event_id
    }

    pub fn authority(&self) -> SemanticContextAuthority {
        self.authority
    }

    pub fn base_projection(&self) -> &ProjectionRef {
        &self.base_projection
    }

    pub fn target_projection(&self) -> &ProjectionRef {
        &self.target_projection
    }

    pub fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    pub fn canonical_payload_bytes(&self) -> &[u8] {
        &self.canonical_payload_bytes
    }

    pub fn serialized_payload_bytes(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.canonical_payload_bytes.len())
            .expect("a canonical JSON value is non-empty")
    }

    pub fn delivery(&self) -> TimelineDeliveryState {
        self.delivery
    }
}

#[derive(Debug, Error)]
pub enum SemanticTimelineEventError {
    #[error("semantic events must advance to a different projection identity")]
    InvalidProjectionTransition,
    #[error("semantic event canonicalization failed: {0}")]
    Canonicalization(#[source] CanonicalProjectionError),
    #[error("canonical semantic payload could not be decoded: {0}")]
    CanonicalPayloadDecode(#[source] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryCheckpointReason {
    PauseResume,
    TimelineOverflow,
    TransportUncertain,
    Cancelled,
    MissingResponse,
    AdapterRestart,
    AuthenticationRecovered,
    PolicyReestablished,
    ProjectionChainMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeltaAvailability {
    Available,
    RecoveryCheckpointRequired { reason: RecoveryCheckpointReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineLimits {
    pub max_events: NonZeroUsize,
    pub max_serialized_payload_bytes: NonZeroUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelinePushOutcome {
    Accepted,
    RecoveryCheckpointRequired,
}

/// Immutable ownership scope for one Agent session epoch's semantic timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimelineAuthority {
    operation_id: Uuid,
    context_id: Uuid,
    session_epoch: NonZeroU64,
}

impl TimelineAuthority {
    pub fn new(operation_id: Uuid, context_id: Uuid, session_epoch: NonZeroU64) -> Self {
        Self {
            operation_id,
            context_id,
            session_epoch,
        }
    }

    pub fn operation_id(self) -> Uuid {
        self.operation_id
    }

    pub fn context_id(self) -> Uuid {
        self.context_id
    }

    pub fn session_epoch(self) -> NonZeroU64 {
        self.session_epoch
    }

    pub fn semantic_context(self) -> SemanticContextAuthority {
        SemanticContextAuthority::new(self.operation_id, self.context_id)
    }
}

/// A finite ordered timeline that never evicts an event not acknowledged by the Agent epoch.
#[derive(Debug)]
pub struct LensSemanticTimeline {
    authority: TimelineAuthority,
    limits: TimelineLimits,
    events: VecDeque<SemanticTimelineEvent>,
    serialized_payload_bytes: usize,
    availability: DeltaAvailability,
    in_flight: Option<AgentTurnKey>,
}

impl LensSemanticTimeline {
    pub fn new(authority: TimelineAuthority, limits: TimelineLimits) -> Self {
        Self {
            authority,
            limits,
            events: VecDeque::new(),
            serialized_payload_bytes: 0,
            availability: DeltaAvailability::Available,
            in_flight: None,
        }
    }

    pub fn availability(&self) -> DeltaAvailability {
        self.availability
    }

    pub fn authority(&self) -> TimelineAuthority {
        self.authority
    }

    pub fn events(&self) -> &VecDeque<SemanticTimelineEvent> {
        &self.events
    }

    pub fn serialized_payload_bytes(&self) -> usize {
        self.serialized_payload_bytes
    }

    pub fn in_flight(&self) -> Option<&AgentTurnKey> {
        self.in_flight.as_ref()
    }

    pub fn push(
        &mut self,
        event: SemanticTimelineEvent,
    ) -> Result<TimelinePushOutcome, TimelineError> {
        if self.availability != DeltaAvailability::Available {
            return Ok(TimelinePushOutcome::RecoveryCheckpointRequired);
        }
        if event.authority.operation_id != self.authority.operation_id {
            return Err(TimelineError::OperationAuthorityMismatch);
        }
        if event.authority.context_id != self.authority.context_id {
            return Err(TimelineError::ContextAuthorityMismatch);
        }
        if let Some(previous) = self.events.back() {
            if previous.target_projection != event.base_projection {
                return Err(TimelineError::ProjectionChainMismatch);
            }
        }

        let event_bytes = event.serialized_payload_bytes().get();
        while self.events.len() >= self.limits.max_events.get()
            || self
                .serialized_payload_bytes
                .checked_add(event_bytes)
                .is_none_or(|total| total > self.limits.max_serialized_payload_bytes.get())
        {
            let may_evict = self.events.front().is_some_and(|event| {
                matches!(event.delivery, TimelineDeliveryState::Acknowledged { .. })
            });
            if !may_evict {
                self.availability = DeltaAvailability::RecoveryCheckpointRequired {
                    reason: RecoveryCheckpointReason::TimelineOverflow,
                };
                return Ok(TimelinePushOutcome::RecoveryCheckpointRequired);
            }
            if let Some(evicted) = self.events.pop_front() {
                self.serialized_payload_bytes = self
                    .serialized_payload_bytes
                    .checked_sub(evicted.serialized_payload_bytes().get())
                    .expect("timeline payload total covers every retained event");
            }
        }

        self.serialized_payload_bytes = self
            .serialized_payload_bytes
            .checked_add(event_bytes)
            .expect("accepted timeline bytes are bounded by the configured maximum");
        self.events.push_back(event);
        Ok(TimelinePushOutcome::Accepted)
    }

    /// Marks the exact ordered pending prefix covered by one delta turn as in-flight.
    pub fn begin_delta_turn(&mut self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        self.validate_turn_authority(key)?;
        if self.availability != DeltaAvailability::Available {
            return Err(TimelineError::RecoveryCheckpointRequired);
        }
        let AgentProjectionCursor::Applied { projection: base } = &key.base_projection else {
            return Err(TimelineError::DeltaRequiresAppliedBase);
        };
        if self.in_flight.is_some() {
            return Err(TimelineError::TurnAlreadyInFlight);
        }

        let first_pending = self
            .events
            .iter()
            .position(|event| event.delivery == TimelineDeliveryState::Pending)
            .ok_or(TimelineError::NoPendingEvents)?;
        if self.events[first_pending].base_projection != *base {
            return Err(TimelineError::ProjectionChainMismatch);
        }

        let target_index = self
            .events
            .iter()
            .enumerate()
            .skip(first_pending)
            .take_while(|(_, event)| event.delivery == TimelineDeliveryState::Pending)
            .find_map(|(index, event)| {
                (event.target_projection == key.target_projection).then_some(index)
            })
            .ok_or(TimelineError::TargetProjectionNotPending)?;

        for event in self.events.range_mut(first_pending..=target_index) {
            event.delivery = TimelineDeliveryState::InFlight {
                turn_id: key.turn_id,
            };
        }
        self.in_flight = Some(key.clone());
        Ok(())
    }

    /// Acknowledges only events previously associated with this exact local turn.
    pub fn acknowledge_turn(&mut self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        self.validate_turn_authority(key)?;
        let Some(in_flight) = self.in_flight.as_ref() else {
            return Err(TimelineError::TurnNotInFlight);
        };
        if in_flight != key {
            return Err(TimelineError::TurnKeyMismatch);
        }
        let matching = self
            .events
            .iter()
            .filter(|event| {
                event.delivery
                    == (TimelineDeliveryState::InFlight {
                        turn_id: key.turn_id,
                    })
            })
            .count();
        if matching == 0 {
            return Err(TimelineError::TurnNotInFlight);
        }
        let final_target_matches = self.events.iter().rev().find(|event| {
            event.delivery
                == (TimelineDeliveryState::InFlight {
                    turn_id: key.turn_id,
                })
        });
        if !final_target_matches
            .is_some_and(|event| event.target_projection == key.target_projection)
        {
            return Err(TimelineError::TargetProjectionMismatch);
        }
        for event in &mut self.events {
            if event.delivery
                == (TimelineDeliveryState::InFlight {
                    turn_id: key.turn_id,
                })
            {
                event.delivery = TimelineDeliveryState::Acknowledged {
                    session_epoch: key.session_epoch,
                };
            }
        }
        self.in_flight = None;
        Ok(())
    }

    /// Latches recovery when the Agent's applied cursor is no longer definitive.
    pub fn require_recovery(&mut self, reason: RecoveryCheckpointReason) {
        if self.availability == DeltaAvailability::Available {
            self.availability = DeltaAvailability::RecoveryCheckpointRequired { reason };
        }
    }

    fn validate_turn_authority(&self, key: &AgentTurnKey) -> Result<(), TimelineError> {
        if key.operation_id != self.authority.operation_id {
            Err(TimelineError::OperationAuthorityMismatch)
        } else if key.context_id != self.authority.context_id {
            Err(TimelineError::ContextAuthorityMismatch)
        } else if key.session_epoch != self.authority.session_epoch {
            Err(TimelineError::EpochAuthorityMismatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error("the turn operation does not own this semantic timeline")]
    OperationAuthorityMismatch,
    #[error("the turn context does not own this semantic timeline")]
    ContextAuthorityMismatch,
    #[error("the turn session epoch does not own this semantic timeline")]
    EpochAuthorityMismatch,
    #[error("semantic timeline projection chain is not contiguous")]
    ProjectionChainMismatch,
    #[error("a delta turn requires an explicitly applied base projection")]
    DeltaRequiresAppliedBase,
    #[error("another timeline turn is already in flight")]
    TurnAlreadyInFlight,
    #[error("the timeline has no pending semantic events")]
    NoPendingEvents,
    #[error("the requested target projection is not an ordered pending target")]
    TargetProjectionNotPending,
    #[error("the requested turn has no in-flight semantic events")]
    TurnNotInFlight,
    #[error("the requested turn key does not equal the active local turn key")]
    TurnKeyMismatch,
    #[error("the in-flight event batch does not end at the turn target projection")]
    TargetProjectionMismatch,
    #[error("the timeline requires a recovery checkpoint")]
    RecoveryCheckpointRequired,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nonzero(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).expect("test value is non-zero")
    }

    fn projection(revision: u64, byte: u8) -> ProjectionRef {
        ProjectionRef::new(
            nonzero(revision),
            format!("{byte:02x}")
                .repeat(32)
                .parse()
                .expect("valid digest"),
        )
    }

    fn source_authority(source_registration_id: u128) -> SourceRegistrationAuthority {
        SourceRegistrationAuthority::new(
            Uuid::from_u128(100),
            Uuid::from_u128(101),
            Uuid::from_u128(source_registration_id),
        )
    }

    fn event(event_id: u128, base: ProjectionRef, target: ProjectionRef) -> SemanticTimelineEvent {
        event_for(
            SemanticContextAuthority::new(Uuid::from_u128(10), Uuid::from_u128(11)),
            event_id,
            base,
            target,
        )
    }

    fn event_for(
        authority: SemanticContextAuthority,
        event_id: u128,
        base: ProjectionRef,
        target: ProjectionRef,
    ) -> SemanticTimelineEvent {
        SemanticTimelineEvent::try_new(
            authority,
            Uuid::from_u128(event_id),
            base,
            target,
            &"change",
        )
        .expect("valid event")
    }

    fn combined_event_payload_bytes(events: &[&SemanticTimelineEvent]) -> NonZeroUsize {
        let bytes = events.iter().fold(0usize, |total, event| {
            total
                .checked_add(event.serialized_payload_bytes().get())
                .expect("test event sizes fit usize")
        });
        NonZeroUsize::new(bytes).expect("test events serialize to non-empty JSON")
    }

    fn turn(turn_id: u128, base: ProjectionRef, target: ProjectionRef) -> AgentTurnKey {
        AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            nonzero(1),
            Uuid::from_u128(turn_id),
            AgentProjectionCursor::Applied { projection: base },
            target,
        )
        .expect("valid turn")
    }

    fn timeline(limits: TimelineLimits) -> LensSemanticTimeline {
        LensSemanticTimeline::new(
            TimelineAuthority::new(Uuid::from_u128(10), Uuid::from_u128(11), nonzero(1)),
            limits,
        )
    }

    #[test]
    fn source_observation_coalesces_bursts_without_parallel_refreshes() {
        let epoch = nonzero(1);
        let authority = source_authority(102);
        let mut coordinator = SourceObservationCoordinator::new(authority, epoch);
        let observer = coordinator.observer_token();
        let other_observer =
            SourceObservationCoordinator::new(source_authority(103), epoch).observer_token();

        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: other_observer
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::RegistrationMismatch {
                    expected: authority,
                    received: other_observer.authority()
                }
            }
        );

        let first = coordinator.apply(SourceObservationEvent::Notification { observer });
        let SourceObservationEffect::CoalescingStarted { token: first_token } = first else {
            panic!("first invalidation must start coalescing");
        };
        assert_eq!(first_token.observer_epoch(), epoch);
        assert_eq!(first_token.work_generation(), nonzero(1));
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::InvalidationCoalesced { token: first_token }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_token }),
            SourceObservationEffect::RefreshStarted { token: first_token }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::DirtyLatched {
                active: first_token
            }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::DirtyAlreadyLatched {
                active: first_token
            }
        );
        let follow_up =
            coordinator.apply(SourceObservationEvent::RefreshCompleted { token: first_token });
        let SourceObservationEffect::FollowUpCoalescingStarted {
            token: follow_up_token,
        } = follow_up
        else {
            panic!("dirty refresh must start one follow-up coalescing interval");
        };
        assert_ne!(follow_up_token, first_token);
        assert_eq!(follow_up_token.work_generation(), nonzero(2));
        assert_eq!(coordinator.refresh(), SourceRefreshState::Coalescing);
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_token }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::WorkTokenMismatch {
                    expected: Some(follow_up_token),
                    received: first_token
                }
            }
        );
    }

    #[test]
    fn pause_resume_and_stop_reject_obsolete_observer_epochs() {
        let first_epoch = nonzero(1);
        let mut coordinator = SourceObservationCoordinator::new(source_authority(102), first_epoch);
        let first_observer = coordinator.observer_token();
        let SourceObservationEffect::CoalescingStarted { token: first_work } =
            coordinator.apply(SourceObservationEvent::Notification {
                observer: first_observer,
            })
        else {
            panic!("notification must create work");
        };
        assert_eq!(
            coordinator.apply(SourceObservationEvent::CoalescingElapsed { token: first_work }),
            SourceObservationEffect::RefreshStarted { token: first_work }
        );
        let paused = coordinator.apply(SourceObservationEvent::Pause);
        assert_eq!(
            paused,
            SourceObservationEffect::Paused {
                observer: coordinator.observer_token()
            }
        );
        assert_eq!(coordinator.observer_epoch(), nonzero(2));
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: first_observer
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: ObservationLifecycle::Paused
                }
            }
        );
        let resumed = coordinator.apply(SourceObservationEvent::Resume);
        let SourceObservationEffect::ResumedWithFullRefresh {
            token: resumed_work,
        } = resumed
        else {
            panic!("resume must start one authoritative full refresh");
        };
        assert_eq!(resumed_work.observer_epoch(), nonzero(3));
        assert_ne!(resumed_work, first_work);
        assert_eq!(coordinator.refresh(), SourceRefreshState::Refreshing);
        assert_eq!(
            coordinator.apply(SourceObservationEvent::RefreshCompleted { token: first_work }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::EpochMismatch {
                    expected: nonzero(3),
                    received: first_work.observer_epoch()
                }
            }
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Stop),
            SourceObservationEffect::Stopped
        );
        assert_eq!(
            coordinator.apply(SourceObservationEvent::Notification {
                observer: resumed_work.observer()
            }),
            SourceObservationEffect::Rejected {
                reason: ObservationRejectionReason::NotWatching {
                    lifecycle: ObservationLifecycle::Stopped
                }
            }
        );
    }

    #[test]
    fn observer_epoch_exhaustion_stops_fail_closed() {
        let mut coordinator =
            SourceObservationCoordinator::new(source_authority(102), nonzero(u64::MAX));

        assert_eq!(
            coordinator.apply(SourceObservationEvent::Pause),
            SourceObservationEffect::StoppedAtGenerationLimit {
                counter: ExhaustedCoordinationCounter::ObserverEpoch
            }
        );
        assert_eq!(coordinator.lifecycle(), ObservationLifecycle::Stopped);
        assert_eq!(coordinator.refresh(), SourceRefreshState::Clean);
        assert_eq!(coordinator.observer_epoch(), nonzero(u64::MAX));

        let mut work_exhausted =
            SourceObservationCoordinator::new(source_authority(102), nonzero(1));
        work_exhausted.last_work_generation = u64::MAX;
        let observer = work_exhausted.observer_token();
        assert_eq!(
            work_exhausted.apply(SourceObservationEvent::Notification { observer }),
            SourceObservationEffect::StoppedAtGenerationLimit {
                counter: ExhaustedCoordinationCounter::WorkGeneration
            }
        );
        assert_eq!(work_exhausted.lifecycle(), ObservationLifecycle::Stopped);
    }

    #[test]
    fn recovery_checkpoint_reason_has_the_complete_stable_wire_vocabulary() {
        let cases = [
            (RecoveryCheckpointReason::PauseResume, "pause_resume"),
            (
                RecoveryCheckpointReason::TimelineOverflow,
                "timeline_overflow",
            ),
            (
                RecoveryCheckpointReason::TransportUncertain,
                "transport_uncertain",
            ),
            (RecoveryCheckpointReason::Cancelled, "cancelled"),
            (
                RecoveryCheckpointReason::MissingResponse,
                "missing_response",
            ),
            (RecoveryCheckpointReason::AdapterRestart, "adapter_restart"),
            (
                RecoveryCheckpointReason::AuthenticationRecovered,
                "authentication_recovered",
            ),
            (
                RecoveryCheckpointReason::PolicyReestablished,
                "policy_reestablished",
            ),
            (
                RecoveryCheckpointReason::ProjectionChainMismatch,
                "projection_chain_mismatch",
            ),
        ];

        for (reason, expected) in cases {
            assert_eq!(
                serde_json::to_value(reason).expect("serialize recovery reason"),
                serde_json::Value::String(expected.to_owned())
            );
        }
    }

    #[test]
    fn semantic_timeline_event_owns_its_exact_canonical_payload_bytes() {
        let event = event(7, projection(1, 1), projection(2, 2));

        assert_eq!(
            event.serialized_payload_bytes().get(),
            event.canonical_payload_bytes().len()
        );
        assert_eq!(event.canonical_payload_bytes(), br#""change""#);
        assert_eq!(event.payload(), "change");
        assert_eq!(event.delivery(), TimelineDeliveryState::Pending);
    }

    #[test]
    fn recovery_reason_is_a_first_terminal_cause_latch() {
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(1).expect("non-zero"),
            max_serialized_payload_bytes: NonZeroUsize::new(1).expect("non-zero"),
        };
        let mut timeline = timeline(limits);

        timeline.require_recovery(RecoveryCheckpointReason::MissingResponse);
        timeline.require_recovery(RecoveryCheckpointReason::Cancelled);

        assert_eq!(
            timeline.availability(),
            DeltaAvailability::RecoveryCheckpointRequired {
                reason: RecoveryCheckpointReason::MissingResponse
            }
        );
    }

    #[test]
    fn turn_key_is_the_exhaustive_candidate_commit_authority() {
        let base = projection(1, 1);
        let target = projection(2, 2);
        let key = turn(12, base, target.clone());

        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::Authoritative
        );
        let other_turn = turn(13, projection(1, 1), target.clone());
        assert_eq!(
            key.candidate_authority(
                &other_turn,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::TurnKeyMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                Uuid::from_u128(99),
                key.context_id,
                key.session_epoch,
                &target
            ),
            CandidateAuthority::OperationMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                Uuid::from_u128(99),
                key.session_epoch,
                &target
            ),
            CandidateAuthority::ContextMismatch
        );
        assert_eq!(
            key.candidate_authority(&key, key.operation_id, key.context_id, nonzero(2), &target),
            CandidateAuthority::EpochMismatch
        );
        assert_eq!(
            key.candidate_authority(
                &key,
                key.operation_id,
                key.context_id,
                key.session_epoch,
                &projection(3, 3)
            ),
            CandidateAuthority::ProjectionMismatch
        );
        assert_eq!(
            key.applied_cursor_after_definitive_response(),
            AgentProjectionCursor::Applied { projection: target }
        );
    }

    #[test]
    fn unacknowledged_timeline_overflow_latches_recovery_without_data_loss() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let p3 = projection(3, 3);
        let p4 = projection(4, 4);
        let first = event(1, p1, p2.clone());
        let second = event(2, p2, p3.clone());
        let third = event(3, p3, p4);
        let max_serialized_payload_bytes = combined_event_payload_bytes(&[&first, &second]);
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(2).expect("non-zero"),
            max_serialized_payload_bytes,
        };
        let mut timeline = timeline(limits);
        timeline.push(first).expect("first event");
        timeline.push(second).expect("second event");
        let before = timeline
            .events()
            .iter()
            .map(SemanticTimelineEvent::event_id)
            .collect::<Vec<_>>();

        assert_eq!(
            timeline.push(third).expect("finite overflow outcome"),
            TimelinePushOutcome::RecoveryCheckpointRequired
        );
        assert_eq!(
            timeline.availability(),
            DeltaAvailability::RecoveryCheckpointRequired {
                reason: RecoveryCheckpointReason::TimelineOverflow
            }
        );
        assert_eq!(
            timeline
                .events()
                .iter()
                .map(SemanticTimelineEvent::event_id)
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(
            timeline.serialized_payload_bytes(),
            max_serialized_payload_bytes.get()
        );
    }

    #[test]
    fn only_acknowledged_prefix_is_evictable() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let p3 = projection(3, 3);
        let p4 = projection(4, 4);
        let first = event(1, p1.clone(), p2.clone());
        let second = event(2, p2.clone(), p3.clone());
        let third = event(3, p3, p4);
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(2).expect("non-zero"),
            max_serialized_payload_bytes: combined_event_payload_bytes(&[&first, &second]),
        };
        let mut timeline = timeline(limits);
        timeline.push(first).expect("first event");
        timeline.push(second).expect("second event");
        let key = turn(20, p1, p2);
        timeline.begin_delta_turn(&key).expect("start first turn");
        timeline
            .acknowledge_turn(&key)
            .expect("acknowledge first turn");

        assert_eq!(
            timeline
                .push(third)
                .expect("acknowledged prefix can be evicted"),
            TimelinePushOutcome::Accepted
        );
        assert_eq!(timeline.events().len(), 2);
        assert_eq!(
            timeline.events().front().expect("event").event_id(),
            Uuid::from_u128(2)
        );
        assert_eq!(timeline.availability(), DeltaAvailability::Available);
    }

    #[test]
    fn timeline_acknowledgement_requires_the_exact_local_turn() {
        let p1 = projection(1, 1);
        let p2 = projection(2, 2);
        let event = event(1, p1.clone(), p2.clone());
        let limits = TimelineLimits {
            max_events: NonZeroUsize::new(4).expect("non-zero"),
            max_serialized_payload_bytes: combined_event_payload_bytes(&[&event]),
        };
        let mut timeline = timeline(limits);
        let foreign_operation_event = event_for(
            SemanticContextAuthority::new(Uuid::from_u128(99), Uuid::from_u128(11)),
            2,
            p1.clone(),
            p2.clone(),
        );
        assert_eq!(
            timeline.push(foreign_operation_event),
            Err(TimelineError::OperationAuthorityMismatch)
        );
        let foreign_context_event = event_for(
            SemanticContextAuthority::new(Uuid::from_u128(10), Uuid::from_u128(99)),
            3,
            p1.clone(),
            p2.clone(),
        );
        assert_eq!(
            timeline.push(foreign_context_event),
            Err(TimelineError::ContextAuthorityMismatch)
        );
        timeline.push(event).expect("event");
        let key = turn(20, p1.clone(), p2.clone());
        let wrong_operation = AgentTurnKey::try_new(
            Uuid::from_u128(99),
            Uuid::from_u128(11),
            nonzero(1),
            Uuid::from_u128(19),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_operation),
            Err(TimelineError::OperationAuthorityMismatch)
        );
        let wrong_context = AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(99),
            nonzero(1),
            Uuid::from_u128(18),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign context turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_context),
            Err(TimelineError::ContextAuthorityMismatch)
        );
        let wrong_epoch = AgentTurnKey::try_new(
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            nonzero(2),
            Uuid::from_u128(17),
            AgentProjectionCursor::Applied {
                projection: p1.clone(),
            },
            p2.clone(),
        )
        .expect("valid but foreign epoch turn");
        assert_eq!(
            timeline.begin_delta_turn(&wrong_epoch),
            Err(TimelineError::EpochAuthorityMismatch)
        );
        assert_eq!(
            timeline.events().front().expect("event").delivery(),
            TimelineDeliveryState::Pending
        );
        timeline.begin_delta_turn(&key).expect("start turn");
        let wrong_key = turn(21, p1, p2);

        assert_eq!(
            timeline.acknowledge_turn(&wrong_key),
            Err(TimelineError::TurnKeyMismatch)
        );
        assert_eq!(
            timeline.events().front().expect("event").delivery(),
            TimelineDeliveryState::InFlight {
                turn_id: key.turn_id
            }
        );
    }
}
