//! Pure application-state admission and reconciliation; the host owns atomic locks/publication.
use crate::{live_sync::ProjectionRef, model::*};
use domain::lens::{LensContext, LensInput, LensMediaPayload, LensTargetSet};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRunKey {
    pub operation_id: Uuid,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSessionIdentity {
    pub operation_id: Uuid,
    pub context_id: Uuid,
    pub config: AppConfig,
}

impl AgentSessionIdentity {
    pub fn admits_reuse(&self, requested: &Self, mailbox_closed: bool) -> bool {
        self == requested && !mailbox_closed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionTurnCompletion {
    Finished,
    Coalesced,
}

/// Capacity-one admission. The host supplies its closed flag while holding its pending
/// lock, then completes displaced/rejected turns only after releasing that lock.
pub fn coalesce_agent_turn<T>(
    pending: &mut Option<T>,
    incoming: T,
    mailbox_closed: bool,
) -> Result<Option<T>, T> {
    if mailbox_closed {
        Err(incoming)
    } else {
        Ok(pending.replace(incoming))
    }
}

/// Finite authority for starting a source read; already-running native work is not abortable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextReadAuthority {
    Initial {
        operation_id: Uuid,
    },
    Refresh {
        operation_id: Uuid,
        context_id: Uuid,
        previous_revision: u64,
    },
}

impl ContextReadAuthority {
    pub fn admits(self, lens: &LensState) -> bool {
        match self {
            Self::Initial { operation_id } => {
                lens.operation_id == Some(operation_id) && lens.stage == LensStage::Extracting
            }
            Self::Refresh {
                operation_id,
                context_id,
                previous_revision,
            } => lens_context_refresh_has_authority(
                lens,
                operation_id,
                context_id,
                previous_revision,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensContextRefreshOutcome {
    Unchanged,
    Updated,
}

pub struct LensContextRefreshCommit {
    pub target_set: LensTargetSet,
    pub context: LensContext,
    pub input: LensInput,
    pub projection: ProjectionRef,
    pub source_health: LensSourceHealth,
    pub outcome: LensContextRefreshOutcome,
}

pub fn advance_revision(snapshot: &mut AppSnapshot) -> Result<(), String> {
    snapshot.revision = next_revision(snapshot)?;
    Ok(())
}

pub fn next_revision(snapshot: &AppSnapshot) -> Result<u32, String> {
    snapshot
        .revision
        .checked_add(1)
        .ok_or_else(|| "application state revision is exhausted".to_string())
}

pub fn lens_context_refresh_has_authority(
    lens: &LensState,
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
) -> bool {
    lens.operation_id == Some(operation_id)
        && lens.context.as_ref().is_some_and(|context| {
            context.context_id == context_id
                && context.revision == expected_previous_context_revision
        })
        && lens
            .live
            .as_ref()
            .is_some_and(|live| live.lifecycle == LensMonitoringLifecycle::Watching)
}

pub fn validate_context_state(operation_id: Uuid, lens: &LensState) -> Result<(), String> {
    if lens.operation_id != Some(operation_id) {
        return Err("canonical Lens state has a mismatched operation identity".into());
    }
    let context = lens
        .context
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a context".to_string())?;
    let input = lens
        .input
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a LensInput".to_string())?;
    let target_set = lens
        .target_set
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a target set".to_string())?;
    if context.revision == 0
        || target_set.selection_id != operation_id
        || input.context_id != context.context_id
        || input.context_revision != context.revision
        || lens.projection.is_none()
    {
        return Err(
            "canonical Lens context, input, and projection revisions are inconsistent".into(),
        );
    }
    if target_set.targets.len() != context.sources.len()
        || target_set
            .targets
            .iter()
            .zip(&context.sources)
            .any(|(target, source)| {
                target.facts_revision == 0
                    || target.facts_revision > context.revision
                    || source.target_id != target.id
                    || source.source.application != target.facts.application_name
                    || source.source.window_title != target.facts.title
                    || source.source.bundle_id != target.identity.bundle_id
                    || source.source.window_id != target.identity.window_id
            })
    {
        return Err(
            "canonical Lens target facts and context source provenance are inconsistent".into(),
        );
    }
    Ok(())
}

pub fn reconcile_lens_after_context_refresh(
    lens: &mut LensState,
    context_revision: u64,
    source_health: LensSourceHealth,
    outcome: LensContextRefreshOutcome,
) {
    match outcome {
        LensContextRefreshOutcome::Updated => {
            let agent_work_is_active =
                matches!(lens.stage, LensStage::Connecting | LensStage::Transforming);
            if !agent_work_is_active {
                lens.pending_representation = None;
                lens.stage = LensStage::Ready;
            }
            lens.error = None;
            let freshness = if source_health == LensSourceHealth::Unavailable {
                LensFreshness::Unverified
            } else if lens.representation.is_some() {
                LensFreshness::Stale
            } else {
                LensFreshness::Checking
            };
            let live = lens
                .live
                .as_mut()
                .expect("a refreshed operation has explicit live state");
            live.health = source_health;
            live.freshness = freshness;
            live.last_outcome = None;
            live.error = None;
        }
        LensContextRefreshOutcome::Unchanged => {
            let representation_is_current = lens
                .representation
                .as_ref()
                .zip(lens.projection.as_ref())
                .is_some_and(|(representation, projection)| {
                    &representation.projection == projection
                });
            if representation_is_current {
                lens.representation
                    .as_mut()
                    .expect("a current representation exists")
                    .context_revision = context_revision;
            }

            let agent_work_is_active =
                matches!(lens.stage, LensStage::Connecting | LensStage::Transforming);
            let preserve_recovery = matches!(
                lens.stage,
                LensStage::AuthenticationRequired | LensStage::Cancelled | LensStage::Failed
            );
            if !preserve_recovery {
                lens.error = None;
            }
            let freshness = if source_health == LensSourceHealth::Unavailable {
                LensFreshness::Unverified
            } else if agent_work_is_active {
                LensFreshness::Checking
            } else if representation_is_current {
                LensFreshness::Current
            } else if lens.representation.is_some() {
                LensFreshness::Stale
            } else {
                LensFreshness::None
            };
            let live = lens
                .live
                .as_mut()
                .expect("a refreshed operation has explicit live state");
            live.health = source_health;
            live.freshness = freshness;
            if agent_work_is_active {
                live.last_outcome = None;
                live.error = None;
            } else if !preserve_recovery {
                live.last_outcome = Some(LensRefreshOutcome::Unchanged);
                live.error = None;
            }
        }
    }
}

pub fn refresh_projection_transition_is_valid(
    previous: &ProjectionRef,
    next: &ProjectionRef,
    outcome: LensContextRefreshOutcome,
) -> bool {
    match outcome {
        LensContextRefreshOutcome::Unchanged => next == previous,
        LensContextRefreshOutcome::Updated => {
            previous
                .revision
                .get()
                .checked_add(1)
                .is_some_and(|revision| revision == next.revision.get())
                && previous.digest != next.digest
        }
    }
}

pub fn validated_payload_map(
    payloads: Vec<LensMediaPayload>,
) -> Result<BTreeMap<String, LensMediaPayload>, String> {
    let mut next = BTreeMap::new();
    let mut uris = BTreeSet::new();
    for payload in payloads {
        if !uris.insert(payload.uri.clone()) {
            return Err("Lens media attachment URI is duplicated".into());
        }
        if next
            .insert(payload.attachment_id.clone(), payload)
            .is_some()
        {
            return Err("Lens media attachment identity is duplicated".into());
        }
    }
    Ok(next)
}

pub fn agent_run_has_authority(
    snapshot: &AppSnapshot,
    key: AgentRunKey,
    expected_config: &AppConfig,
) -> bool {
    &snapshot.config == expected_config
        && snapshot.lens.operation_id == Some(key.operation_id)
        && snapshot
            .lens
            .agent
            .as_ref()
            .is_some_and(|run| run.run_id == key.run_id)
}

pub fn validate_refresh_identity(
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
    refresh: &LensContextRefreshCommit,
) -> Result<(), String> {
    let expected_context_revision = expected_previous_context_revision
        .checked_add(1)
        .ok_or_else(|| "Lens context revision is exhausted".to_string())?;
    if refresh.context.context_id != context_id
        || refresh.context.revision != expected_context_revision
        || refresh.target_set.selection_id != operation_id
    {
        return Err("refreshed Lens target/context identity or revision is inconsistent".into());
    }
    Ok(())
}

/// Build a complete candidate from the host's latest locked state. No publication occurs here.
pub fn prepare_context_refresh(
    latest: &LensState,
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
    refresh: LensContextRefreshCommit,
) -> Result<Option<LensState>, String> {
    validate_refresh_identity(
        operation_id,
        context_id,
        expected_previous_context_revision,
        &refresh,
    )?;
    if !lens_context_refresh_has_authority(
        latest,
        operation_id,
        context_id,
        expected_previous_context_revision,
    ) {
        return Ok(None);
    }
    let previous_projection = latest
        .projection
        .as_ref()
        .ok_or_else(|| "the current Lens operation has no Agent projection".to_string())?;
    let previous_target_set = latest
        .target_set
        .as_ref()
        .ok_or_else(|| "the current Lens operation has no fixed target set".to_string())?;
    if !previous_target_set.has_same_identity(&refresh.target_set) {
        return Err("refreshed Lens target identity changed during facts refresh".into());
    }
    if !refresh_projection_transition_is_valid(
        previous_projection,
        &refresh.projection,
        refresh.outcome,
    ) {
        return Err("refreshed Lens projection transition is inconsistent".into());
    }

    let mut next = latest.clone();
    let context_revision = refresh.context.revision;
    next.target_set = Some(refresh.target_set);
    next.context = Some(refresh.context);
    next.input = Some(refresh.input);
    next.projection = Some(refresh.projection);
    reconcile_lens_after_context_refresh(
        &mut next,
        context_revision,
        refresh.source_health,
        refresh.outcome,
    );
    validate_context_state(operation_id, &next)?;

    Ok(Some(next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_mailbox_coalesces_one_pending_turn_and_rejects_closed_admission() {
        let mut pending = None;
        assert_eq!(coalesce_agent_turn(&mut pending, 1, false), Ok(None));
        assert_eq!(pending, Some(1));
        assert_eq!(coalesce_agent_turn(&mut pending, 2, false), Ok(Some(1)));
        assert_eq!(pending, Some(2));
        assert_eq!(coalesce_agent_turn(&mut pending, 3, true), Err(3));
        assert_eq!(pending, Some(2));
        assert_eq!(pending.take(), Some(2));
        assert_eq!(coalesce_agent_turn(&mut pending, 4, true), Err(4));
        assert_eq!(pending, None);
    }

    #[test]
    fn agent_session_reuse_requires_exact_identity_and_an_open_mailbox() {
        let identity = AgentSessionIdentity {
            operation_id: Uuid::from_u128(1),
            context_id: Uuid::from_u128(2),
            config: AppConfig::new("/fixture".into()),
        };
        assert!(identity.admits_reuse(&identity, false));
        assert!(!identity.admits_reuse(&identity, true));
        for axis in 0..3 {
            let mut changed = identity.clone();
            match axis {
                0 => changed.operation_id = Uuid::from_u128(3),
                1 => changed.context_id = Uuid::from_u128(3),
                2 => changed.config.working_directory = "/changed".into(),
                _ => unreachable!(),
            }
            assert!(!identity.admits_reuse(&changed, false));
        }
    }

    #[test]
    fn observation_refresh_admission_reads_only_the_current_watching_context() {
        use crate::observation::refresh_revision;
        let original = canonical_state(7);
        let operation_id = original.operation_id.unwrap();
        let context_id = original.context.as_ref().unwrap().context_id;
        assert_eq!(
            refresh_revision(&original, operation_id, context_id),
            Some(7)
        );
        for axis in 0..6 {
            let mut stale = original.clone();
            match axis {
                0 => stale.operation_id = None,
                1 => stale.operation_id = Some(Uuid::from_u128(3)),
                2 => stale.context = None,
                3 => stale.context.as_mut().unwrap().context_id = Uuid::from_u128(3),
                4 => stale.live = None,
                5 => stale.live.as_mut().unwrap().lifecycle = LensMonitoringLifecycle::Paused,
                _ => unreachable!(),
            }
            assert_eq!(refresh_revision(&stale, operation_id, context_id), None);
        }
        let mut stopped = original;
        stopped.live.as_mut().unwrap().lifecycle = LensMonitoringLifecycle::Stopped;
        assert_eq!(refresh_revision(&stopped, operation_id, context_id), None);
    }

    #[test]
    fn observation_coverage_preserves_failure_and_degradation_precedence() {
        use crate::observation::ObservationCoverage;
        let mut lens = canonical_state(7);
        let mut coverage = ObservationCoverage {
            expected_sources: 2,
            observing_sources: 2,
            has_registration_diagnostics: false,
            failures: vec![],
        };
        let original = lens.clone();
        coverage.apply_to(&mut lens);
        assert_eq!(lens, original);
        coverage.has_registration_diagnostics = true;
        coverage.apply_to(&mut lens);
        assert_eq!(
            lens.live.as_ref().unwrap().health,
            LensSourceHealth::Degraded
        );
        coverage.observing_sources = 0;
        coverage.failures = vec!["first".into(), "second".into()];
        coverage.apply_to(&mut lens);
        let live = lens.live.as_ref().unwrap();
        assert_eq!(live.health, LensSourceHealth::Unavailable);
        assert_eq!(live.freshness, LensFreshness::Unverified);
        assert_eq!(live.error.as_deref(), Some("first; second"));
        coverage.observing_sources = 2;
        coverage.apply_to(&mut lens);
        assert_eq!(
            lens.live.as_ref().unwrap().health,
            LensSourceHealth::Unavailable
        );
        lens.live = None;
        let without_live = lens.clone();
        coverage.apply_to(&mut lens);
        assert_eq!(lens, without_live);
    }

    fn canonical_state(revision: u64) -> LensState {
        use domain::{lens::LensTargetCapture, model::ExtractedNode};
        let targets = target_set(Uuid::from_u128(1), "Current");
        let capture = ExtractionResult {
            quality: ExtractionQuality::Full,
            resolved_window: None,
            nodes: vec![ExtractedNode {
                id: "root".into(),
                parent_id: None,
                order: 0,
                depth: 0,
                role: Some("AXWindow".into()),
                subrole: None,
                title: Some("Current".into()),
                value: None,
                description: None,
                bounds: None,
                children: vec![],
                resource_refs: vec![],
            }],
            text: "Current".into(),
            metrics: ExtractionMetrics::default(),
            diagnostics: vec![],
        };
        let context = LensContext::from_captures_at_revision(
            Uuid::from_u128(2),
            revision,
            BTreeMap::from([(targets.targets[0].id.clone(), revision)]),
            &targets,
            vec![LensTargetCapture {
                accessibility: capture,
                media: Default::default(),
            }],
        )
        .unwrap();
        let lens = LensState {
            operation_id: Some(targets.selection_id),
            stage: LensStage::Completed,
            input: Some(LensInput::from_context(&context).unwrap()),
            context: Some(context),
            target_set: Some(targets),
            projection: Some(projection_ref(1, 'a')),
            representation: Some(LensRepresentation {
                context_id: Uuid::from_u128(2),
                ..representation(projection_ref(1, 'a'), 1)
            }),
            live: Some(live_state(LensFreshness::Current, None, None)),
            ..LensState::default()
        };
        validate_context_state(Uuid::from_u128(1), &lens).unwrap();
        lens
    }

    #[test]
    fn initial_source_reads_require_the_exact_extracting_operation() {
        let operation_id = Uuid::from_u128(1);
        let authority = ContextReadAuthority::Initial { operation_id };
        for stage in [
            LensStage::Idle,
            LensStage::Selecting,
            LensStage::Extracting,
            LensStage::Ready,
            LensStage::Connecting,
            LensStage::AuthenticationRequired,
            LensStage::Transforming,
            LensStage::Completed,
            LensStage::Cancelled,
            LensStage::Failed,
        ] {
            let mut lens = LensState {
                operation_id: Some(operation_id),
                stage,
                ..LensState::default()
            };
            assert_eq!(authority.admits(&lens), stage == LensStage::Extracting);
            lens.operation_id = Some(Uuid::from_u128(2));
            assert!(!authority.admits(&lens));
            lens.operation_id = None;
            assert!(!authority.admits(&lens));
        }
    }

    #[test]
    fn refresh_source_reads_require_current_context_revision_and_watching_lifecycle() {
        let lens = canonical_state(3);
        let authority = ContextReadAuthority::Refresh {
            operation_id: lens.operation_id.unwrap(),
            context_id: lens.context.as_ref().unwrap().context_id,
            previous_revision: 3,
        };
        assert!(authority.admits(&lens));
        for lifecycle in [
            LensMonitoringLifecycle::Paused,
            LensMonitoringLifecycle::Stopped,
        ] {
            let mut revoked = lens.clone();
            revoked.live.as_mut().unwrap().lifecycle = lifecycle;
            assert!(!authority.admits(&revoked));
        }
        let mut revoked = lens.clone();
        revoked.context.as_mut().unwrap().revision = 4;
        assert!(!authority.admits(&revoked));
        revoked = lens.clone();
        revoked.context.as_mut().unwrap().context_id = Uuid::from_u128(99);
        assert!(!authority.admits(&revoked));
        revoked = lens.clone();
        revoked.operation_id = Some(Uuid::from_u128(99));
        assert!(!authority.admits(&revoked));
        revoked = lens.clone();
        revoked.context = None;
        assert!(!authority.admits(&revoked));
        revoked = lens;
        revoked.live = None;
        assert!(!authority.admits(&revoked));
    }

    fn refresh_candidate() -> LensContextRefreshCommit {
        let lens = canonical_state(2);
        LensContextRefreshCommit {
            target_set: lens.target_set.unwrap(),
            context: lens.context.unwrap(),
            input: lens.input.unwrap(),
            projection: lens.projection.unwrap(),
            source_health: LensSourceHealth::Healthy,
            outcome: LensContextRefreshOutcome::Unchanged,
        }
    }

    #[test]
    fn complete_refresh_preparation_preserves_latest_state_and_never_publishes() {
        let mut latest = canonical_state(1);
        latest.output_blocks = vec![LensOutputBlock::Markdown {
            message_id: Some("latest".into()),
            text: "Latest Agent output".into(),
        }];
        let before = latest.clone();
        let next = prepare_context_refresh(
            &latest,
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            1,
            refresh_candidate(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(latest, before);
        assert_eq!(next.context.as_ref().unwrap().revision, 2);
        assert_eq!(next.output_blocks, latest.output_blocks);
        assert_eq!(next.stage, LensStage::Completed);
        assert_eq!(next.representation.as_ref().unwrap().context_revision, 2);
        assert_eq!(
            next.live.as_ref().unwrap().freshness,
            LensFreshness::Current
        );
        validate_context_state(Uuid::from_u128(1), &next).unwrap();
    }

    #[test]
    fn complete_refresh_rejects_stale_authority_without_changing_latest_state() {
        for change in [
            (|lens: &mut LensState| lens.operation_id = Some(Uuid::from_u128(9)))
                as fn(&mut LensState),
            |lens| lens.context.as_mut().unwrap().context_id = Uuid::from_u128(9),
            |lens| lens.context.as_mut().unwrap().revision = 2,
            |lens| lens.live.as_mut().unwrap().lifecycle = LensMonitoringLifecycle::Paused,
            |lens| lens.live.as_mut().unwrap().lifecycle = LensMonitoringLifecycle::Stopped,
            |lens| lens.live = None,
        ] {
            let mut latest = canonical_state(1);
            change(&mut latest);
            let before = latest.clone();
            assert!(prepare_context_refresh(
                &latest,
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                1,
                refresh_candidate()
            )
            .unwrap()
            .is_none());
            assert_eq!(latest, before);
        }
    }

    #[test]
    fn complete_refresh_rejects_mismatched_context_target_projection_and_provenance() {
        let latest = canonical_state(1);
        for change in [
            (|next: &mut LensContextRefreshCommit| next.context.context_id = Uuid::from_u128(9))
                as fn(&mut LensContextRefreshCommit),
            |next| next.context.revision = 3,
            |next| next.target_set.selection_id = Uuid::from_u128(9),
            |next| next.target_set.targets[0].identity.window_id += 1,
            |next| next.projection = projection_ref(2, 'b'),
            |next| next.outcome = LensContextRefreshOutcome::Updated,
            |next| next.input.context_revision = 1,
            |next| next.context.sources[0].source.window_title = "Wrong provenance".into(),
        ] {
            let mut candidate = refresh_candidate();
            change(&mut candidate);
            let before = latest.clone();
            assert!(prepare_context_refresh(
                &latest,
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                1,
                candidate
            )
            .is_err());
            assert_eq!(latest, before);
        }
    }

    #[test]
    fn exhausted_snapshot_and_context_revisions_never_wrap() {
        let mut snapshot = AppSnapshot::new(AppConfig::new("/fixture".into()));
        snapshot.revision = u32::MAX;
        assert!(advance_revision(&mut snapshot).is_err());
        assert_eq!(snapshot.revision, u32::MAX);
        assert!(validate_refresh_identity(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            u64::MAX,
            &refresh_candidate()
        )
        .is_err());
    }

    #[test]
    fn payload_map_rejects_duplicate_identity_and_uri_independently() {
        let first = LensMediaPayload {
            attachment_id: "one".into(),
            uri: "lens://fixture/one".into(),
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };
        for duplicate in [
            LensMediaPayload {
                attachment_id: "two".into(),
                ..first.clone()
            },
            LensMediaPayload {
                uri: "lens://fixture/two".into(),
                ..first.clone()
            },
        ] {
            assert!(validated_payload_map(vec![first.clone(), duplicate]).is_err());
        }
        assert_eq!(
            validated_payload_map(vec![first.clone()])
                .unwrap()
                .get("one"),
            Some(&first)
        );
    }
    fn projection_ref(revision: u64, digit: char) -> ProjectionRef {
        ProjectionRef::new(
            std::num::NonZeroU64::new(revision).expect("non-zero projection revision"),
            serde_json::from_str(&format!("\"{}\"", digit.to_string().repeat(64)))
                .expect("valid projection digest"),
        )
    }

    fn live_state(
        freshness: LensFreshness,
        last_outcome: Option<LensRefreshOutcome>,
        error: Option<&str>,
    ) -> crate::model::LensLiveState {
        crate::model::LensLiveState {
            lifecycle: LensMonitoringLifecycle::Watching,
            health: LensSourceHealth::Healthy,
            freshness,
            agent_refresh_interval_seconds: crate::model::LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
            last_outcome,
            error: error.map(str::to_string),
        }
    }

    fn representation(
        projection: ProjectionRef,
        context_revision: u64,
    ) -> crate::model::LensRepresentation {
        crate::model::LensRepresentation {
            representation_id: Uuid::from_u128(10),
            context_id: Uuid::nil(),
            context_revision,
            projection,
            run_id: Uuid::from_u128(11),
            output_blocks: vec![crate::model::LensOutputBlock::Markdown {
                message_id: None,
                text: "Settled representation".into(),
            }],
        }
    }

    fn context(context_id: Uuid, revision: u64) -> LensContext {
        LensContext {
            schema_version: domain::lens::LENS_CONTEXT_SCHEMA_VERSION,
            context_id,
            revision,
            sources: vec![],
            media: vec![],
            media_omissions: vec![],
            quality: crate::model::ExtractionQuality::Unavailable,
            diagnostics: vec![],
        }
    }

    fn target_set(operation_id: Uuid, title: &str) -> LensTargetSet {
        LensTargetSet::try_new(
            operation_id,
            vec![crate::model::SelectedWindow {
                identity: crate::model::WindowIdentity {
                    window_id: 42,
                    bundle_id: "example.browser".into(),
                    pid: 100,
                },
                facts: crate::model::WindowObservableFacts {
                    title: title.into(),
                    application_name: "Browser".into(),
                    frame: crate::model::Bounds {
                        x: 10.0,
                        y: 20.0,
                        width: 800.0,
                        height: 600.0,
                    },
                },
            }],
        )
        .expect("valid target set")
    }

    #[test]
    fn active_run_authority_is_independent_from_the_latest_projection() {
        let operation_id = Uuid::from_u128(1);
        let run_id = Uuid::from_u128(2);
        let config = AppConfig::new(std::path::PathBuf::from("/fixture"));
        let projection = |revision, digit: char| {
            ProjectionRef::new(
                std::num::NonZeroU64::new(revision).expect("non-zero projection revision"),
                serde_json::from_str(&format!("\"{}\"", digit.to_string().repeat(64)))
                    .expect("valid projection digest"),
            )
        };
        let active_projection = projection(1, 'a');
        let latest_projection = projection(2, 'b');
        let mut snapshot = AppSnapshot::new(config.clone());
        snapshot.lens.operation_id = Some(operation_id);
        snapshot.lens.projection = Some(latest_projection);
        snapshot.lens.agent = Some(crate::model::AgentRunState {
            run_id,
            input_projection: Some(active_projection),
            kind: crate::model::AgentKind::Codex,
            adapter_name: "test-agent".into(),
            adapter_version: "1.0.0".into(),
            session_id: Some("session".into()),
            session_mode_id: Some("read-only".into()),
            auth_methods: vec![],
            received_updates: 0,
            stop_reason: None,
            authentication_message: None,
        });
        let key = AgentRunKey {
            operation_id,
            run_id,
        };

        assert!(agent_run_has_authority(&snapshot, key, &config));
        snapshot.lens.agent.as_mut().expect("active run").run_id = Uuid::from_u128(3);
        assert!(!agent_run_has_authority(&snapshot, key, &config));
    }

    #[test]
    fn stale_refresh_has_no_authority_to_replace_current_observable_facts() {
        let operation_id = Uuid::from_u128(20);
        let context_id = Uuid::from_u128(21);
        let canonical_targets = target_set(operation_id, "Current title");
        let stale_targets = target_set(operation_id, "Stale worker title");
        let lens = LensState {
            operation_id: Some(operation_id),
            target_set: Some(canonical_targets.clone()),
            context: Some(context(context_id, 2)),
            live: Some(live_state(LensFreshness::Current, None, None)),
            ..LensState::default()
        };

        assert!(!lens_context_refresh_has_authority(
            &lens,
            operation_id,
            context_id,
            1,
        ));
        assert!(lens_context_refresh_has_authority(
            &lens,
            operation_id,
            context_id,
            2,
        ));
        assert_eq!(lens.target_set.as_ref(), Some(&canonical_targets));
        assert_ne!(lens.target_set.as_ref(), Some(&stale_targets));
    }

    #[test]
    fn unchanged_refresh_preserves_stale_recovery_state_and_representation_provenance() {
        let settled_projection = projection_ref(1, 'a');
        let latest_projection = projection_ref(2, 'b');
        let cases = [
            (
                LensStage::AuthenticationRequired,
                None,
                Some(LensRefreshOutcome::Failed),
                Some("authentication required"),
            ),
            (LensStage::Cancelled, None, None, None),
            (
                LensStage::Failed,
                Some("Agent failed"),
                Some(LensRefreshOutcome::Failed),
                Some("Agent failed"),
            ),
        ];

        for (stage, state_error, last_outcome, live_error) in cases {
            let mut lens = LensState {
                stage,
                projection: Some(latest_projection.clone()),
                representation: Some(representation(settled_projection.clone(), 4)),
                live: Some(live_state(LensFreshness::Stale, last_outcome, live_error)),
                error: state_error.map(str::to_string),
                ..LensState::default()
            };

            reconcile_lens_after_context_refresh(
                &mut lens,
                5,
                LensSourceHealth::Healthy,
                LensContextRefreshOutcome::Unchanged,
            );

            let retained = lens
                .representation
                .as_ref()
                .expect("retained representation");
            assert_eq!(retained.projection, settled_projection);
            assert_eq!(retained.context_revision, 4);
            assert_eq!(lens.stage, stage);
            assert_eq!(lens.error.as_deref(), state_error);
            let live = lens.live.as_ref().expect("live state");
            assert_eq!(live.freshness, LensFreshness::Stale);
            assert_eq!(live.last_outcome, last_outcome);
            assert_eq!(live.error.as_deref(), live_error);
        }
    }

    #[test]
    fn refresh_projection_transition_requires_exact_outcome_and_revision() {
        let first = projection_ref(1, 'a');
        let same = first.clone();
        let next = projection_ref(2, 'b');
        let skipped = projection_ref(3, 'c');
        let same_digest_next_revision = ProjectionRef::new(
            std::num::NonZeroU64::new(2).expect("non-zero projection revision"),
            first.digest.clone(),
        );

        assert!(refresh_projection_transition_is_valid(
            &first,
            &same,
            LensContextRefreshOutcome::Unchanged,
        ));
        assert!(refresh_projection_transition_is_valid(
            &first,
            &next,
            LensContextRefreshOutcome::Updated,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &next,
            LensContextRefreshOutcome::Unchanged,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &skipped,
            LensContextRefreshOutcome::Updated,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &same_digest_next_revision,
            LensContextRefreshOutcome::Updated,
        ));
    }

    #[test]
    fn unchanged_refresh_advances_only_matching_representation_provenance() {
        let projection = projection_ref(2, 'b');
        let mut lens = LensState {
            stage: LensStage::Completed,
            projection: Some(projection.clone()),
            representation: Some(representation(projection, 4)),
            live: Some(live_state(
                LensFreshness::Checking,
                Some(LensRefreshOutcome::Failed),
                Some("obsolete error"),
            )),
            error: Some("obsolete error".into()),
            ..LensState::default()
        };

        reconcile_lens_after_context_refresh(
            &mut lens,
            5,
            LensSourceHealth::Healthy,
            LensContextRefreshOutcome::Unchanged,
        );

        assert_eq!(
            lens.representation
                .as_ref()
                .expect("current representation")
                .context_revision,
            5
        );
        assert_eq!(lens.stage, LensStage::Completed);
        assert!(lens.error.is_none());
        let live = lens.live.as_ref().expect("live state");
        assert_eq!(live.freshness, LensFreshness::Current);
        assert_eq!(live.last_outcome, Some(LensRefreshOutcome::Unchanged));
        assert!(live.error.is_none());
    }

    #[test]
    fn updated_refresh_preserves_active_work_and_reopens_terminal_recovery() {
        for stage in [LensStage::Connecting, LensStage::Transforming] {
            let mut lens = LensState {
                stage,
                live: Some(live_state(
                    LensFreshness::None,
                    Some(LensRefreshOutcome::Updated),
                    None,
                )),
                ..LensState::default()
            };
            reconcile_lens_after_context_refresh(
                &mut lens,
                5,
                LensSourceHealth::Healthy,
                LensContextRefreshOutcome::Updated,
            );

            assert_eq!(lens.stage, stage);
            assert_eq!(
                lens.live.as_ref().map(|live| live.freshness),
                Some(LensFreshness::Checking)
            );
            assert_eq!(lens.live.as_ref().and_then(|live| live.last_outcome), None);
        }

        let settled_projection = projection_ref(1, 'a');
        let latest_projection = projection_ref(2, 'b');
        let mut failed = LensState {
            stage: LensStage::Failed,
            projection: Some(latest_projection),
            representation: Some(representation(settled_projection, 4)),
            live: Some(live_state(
                LensFreshness::Stale,
                Some(LensRefreshOutcome::Failed),
                Some("Agent failed"),
            )),
            error: Some("Agent failed".into()),
            ..LensState::default()
        };
        reconcile_lens_after_context_refresh(
            &mut failed,
            5,
            LensSourceHealth::Healthy,
            LensContextRefreshOutcome::Updated,
        );

        assert_eq!(failed.stage, LensStage::Ready);
        assert!(failed.error.is_none());
        let live = failed.live.as_ref().expect("live state");
        assert_eq!(live.freshness, LensFreshness::Stale);
        assert_eq!(live.last_outcome, None);
        assert!(live.error.is_none());
    }
}
