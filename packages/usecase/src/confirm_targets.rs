//! Confirmation ordering, independent of native presentation and task drivers.

use crate::model::{LensStage, LensState, LensTargetSelection, LensTargetSelectionStage};
use domain::lens::LensTargetSet;
use std::future::Future;
use uuid::Uuid;

pub const OPERATION_SUPERSEDED: &str = "Lens operation was superseded by a newer selection";

/// App-owned effects required by confirmation, not a general platform adapter.
/// State writes and extraction commits must check the supplied operation identity.
pub trait ConfirmationHost {
    fn selection(&self, operation: Uuid) -> Result<LensTargetSelection, String>;
    fn dismiss_preview(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn replace_state(&self, operation: Uuid, next: LensState) -> Result<bool, String>;
    fn extract_registered(
        &self,
        operation: Uuid,
        targets: LensTargetSet,
    ) -> impl Future<Output = Result<LensState, String>> + Send;
    fn start_observation(&self, operation: Uuid) -> Result<(), String>;
    fn mark_observation_unavailable(&self, operation: Uuid, error: String) -> Result<(), String>;
    fn transform(&self, operation: Uuid) -> impl Future<Output = Result<LensState, String>> + Send;
    fn request_refresh(&self, operation: Uuid);
}

pub async fn confirm_targets(
    host: &impl ConfirmationHost,
    operation: Uuid,
) -> Result<LensState, String> {
    let selection = host.selection(operation)?;
    if selection.stage != LensTargetSelectionStage::Reviewing {
        return Err("Lens target selection cannot be confirmed while the picker is active".into());
    }
    let windows = selection
        .items
        .iter()
        .map(|item| item.window.clone())
        .collect();
    let targets = LensTargetSet::try_new(operation, windows).map_err(|error| error.to_string())?;
    host.dismiss_preview().await?;
    // Dismissal awaits a native window animation while the picker webview is still live, so
    // the reviewed set can change underneath this call. `replace_state` only compares the
    // operation id, so without re-reading here a removal completed during the animation is
    // silently discarded — and a cancellation that kept the same id would be overwritten.
    if host.selection(operation)? != selection {
        return Err(OPERATION_SUPERSEDED.into());
    }
    let extracting = LensState {
        operation_id: Some(operation),
        stage: LensStage::Extracting,
        target_set: Some(targets.clone()),
        ..LensState::default()
    };
    if !host.replace_state(operation, extracting)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    let ready = host.extract_registered(operation, targets).await?;
    if ready.stage != LensStage::Ready {
        return Ok(ready);
    }
    if let Err(error) = host.start_observation(operation) {
        host.mark_observation_unavailable(operation, error)?;
    }
    let transformed = host.transform(operation).await?;
    host.request_refresh(operation);
    Ok(transformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        LensFreshness, LensLiveState, LensMonitoringLifecycle, LensSourceHealth,
        LensTargetSelectionItem, SelectedWindow, LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
    };
    use std::sync::Mutex;

    const OPERATION: Uuid = Uuid::from_u128(1);
    const REPLACEMENT: Uuid = Uuid::from_u128(2);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Effect {
        Selection,
        Dismiss,
        BeginExtraction,
        Extract,
        Observe,
        Degrade,
        Transform,
        Refresh,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Scenario {
        Success,
        Picking,
        EmptySelection,
        DismissFailure,
        SelectionChangedDuringDismiss,
        SupersededDuringDismiss,
        SupersededDuringExtraction,
        ExtractionFailure,
        UnusableExtraction,
        ObserverFailure,
        TransformFailure,
    }

    struct Script {
        state: LensState,
        effects: Vec<Effect>,
        dismissed: bool,
    }

    struct Host {
        scenario: Scenario,
        script: Mutex<Script>,
    }

    impl Host {
        fn new(scenario: Scenario) -> Self {
            Self {
                scenario,
                script: Mutex::new(Script {
                    state: LensState {
                        operation_id: Some(OPERATION),
                        stage: LensStage::Selecting,
                        ..LensState::default()
                    },
                    effects: Vec::new(),
                    dismissed: false,
                }),
            }
        }

        fn record(&self, effect: Effect) {
            self.script.lock().unwrap().effects.push(effect);
        }

        fn supersede(&self) {
            self.script.lock().unwrap().state = LensState {
                operation_id: Some(REPLACEMENT),
                stage: LensStage::Selecting,
                ..LensState::default()
            };
        }
    }

    impl ConfirmationHost for Host {
        fn selection(&self, _: Uuid) -> Result<LensTargetSelection, String> {
            self.record(Effect::Selection);
            // Removing the reviewed window while the dismissal animation runs keeps the
            // operation id, so only re-reading the selection can detect it.
            let removed = self.scenario == Scenario::SelectionChangedDuringDismiss
                && self.script.lock().unwrap().dismissed;
            let window: SelectedWindow = serde_json::from_value(serde_json::json!({
                "window_id":7,"bundle_id":"example.browser","pid":42,
                "title":"Document","application_name":"Browser",
                "frame":{"x":0.0,"y":0.0,"width":100.0,"height":100.0}
            }))
            .unwrap();
            Ok(LensTargetSelection {
                selection_id: OPERATION,
                stage: if self.scenario == Scenario::Picking {
                    LensTargetSelectionStage::Picking
                } else {
                    LensTargetSelectionStage::Reviewing
                },
                anchor: Some(window.facts.frame),
                maximum_targets: domain::lens::MAX_LENS_TARGETS,
                items: if self.scenario == Scenario::EmptySelection || removed {
                    Vec::new()
                } else {
                    vec![LensTargetSelectionItem {
                        id: "macos:example.browser:7".into(),
                        window,
                        preview_uri: None,
                        preview_error: None,
                    }]
                },
                notice: None,
            })
        }

        async fn dismiss_preview(&self) -> Result<(), String> {
            self.record(Effect::Dismiss);
            tokio::task::yield_now().await;
            self.script.lock().unwrap().dismissed = true;
            if self.scenario == Scenario::SupersededDuringDismiss {
                self.supersede();
            }
            if self.scenario == Scenario::DismissFailure {
                return Err("dismiss failed".into());
            }
            Ok(())
        }

        fn replace_state(&self, operation: Uuid, next: LensState) -> Result<bool, String> {
            self.record(Effect::BeginExtraction);
            let mut script = self.script.lock().unwrap();
            if script.state.operation_id != Some(operation) {
                return Ok(false);
            }
            script.state = next;
            Ok(true)
        }

        async fn extract_registered(
            &self,
            operation: Uuid,
            _: LensTargetSet,
        ) -> Result<LensState, String> {
            self.record(Effect::Extract);
            tokio::task::yield_now().await;
            if self.scenario == Scenario::SupersededDuringExtraction {
                self.supersede();
            }
            let mut script = self.script.lock().unwrap();
            if script.state.operation_id != Some(operation) {
                return Err(OPERATION_SUPERSEDED.into());
            }
            if matches!(
                self.scenario,
                Scenario::ExtractionFailure | Scenario::UnusableExtraction
            ) {
                script.state.stage = LensStage::Failed;
                script.state.error = Some("extraction failed".into());
                return if self.scenario == Scenario::ExtractionFailure {
                    Err("extraction failed".into())
                } else {
                    Ok(script.state.clone())
                };
            }
            script.state.stage = LensStage::Ready;
            script.state.live = Some(LensLiveState {
                lifecycle: LensMonitoringLifecycle::Watching,
                health: LensSourceHealth::Healthy,
                freshness: LensFreshness::None,
                agent_refresh_interval_seconds: LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
                last_outcome: None,
                error: None,
            });
            Ok(script.state.clone())
        }

        fn start_observation(&self, _: Uuid) -> Result<(), String> {
            self.record(Effect::Observe);
            if self.scenario == Scenario::ObserverFailure {
                return Err("observer failed".into());
            }
            Ok(())
        }

        fn mark_observation_unavailable(&self, _: Uuid, error: String) -> Result<(), String> {
            self.record(Effect::Degrade);
            let mut script = self.script.lock().unwrap();
            let live = script.state.live.as_mut().unwrap();
            live.health = LensSourceHealth::Unavailable;
            live.freshness = LensFreshness::Unverified;
            live.error = Some(error);
            Ok(())
        }

        async fn transform(&self, _: Uuid) -> Result<LensState, String> {
            self.record(Effect::Transform);
            if self.scenario == Scenario::TransformFailure {
                return Err("transform failed".into());
            }
            let mut script = self.script.lock().unwrap();
            script.state.stage = LensStage::Transforming;
            Ok(script.state.clone())
        }

        fn request_refresh(&self, _: Uuid) {
            self.record(Effect::Refresh);
        }
    }

    #[tokio::test]
    async fn confirmation_preserves_effect_order_and_terminal_state() {
        use Effect::*;
        use Scenario::*;
        for (scenario, expected_effects, expected_stage, expected_error) in [
            (
                Success,
                vec![
                    Selection,
                    Dismiss,
                    Selection,
                    BeginExtraction,
                    Extract,
                    Observe,
                    Transform,
                    Refresh,
                ],
                LensStage::Transforming,
                None,
            ),
            (
                Picking,
                vec![Selection],
                LensStage::Selecting,
                Some("Lens target selection cannot be confirmed while the picker is active"),
            ),
            (
                DismissFailure,
                vec![Selection, Dismiss],
                LensStage::Selecting,
                Some("dismiss failed"),
            ),
            (
                SelectionChangedDuringDismiss,
                vec![Selection, Dismiss, Selection],
                LensStage::Selecting,
                Some(OPERATION_SUPERSEDED),
            ),
            (
                SupersededDuringDismiss,
                vec![Selection, Dismiss, Selection, BeginExtraction],
                LensStage::Selecting,
                Some(OPERATION_SUPERSEDED),
            ),
            (
                SupersededDuringExtraction,
                vec![Selection, Dismiss, Selection, BeginExtraction, Extract],
                LensStage::Selecting,
                Some(OPERATION_SUPERSEDED),
            ),
            (
                ExtractionFailure,
                vec![Selection, Dismiss, Selection, BeginExtraction, Extract],
                LensStage::Failed,
                Some("extraction failed"),
            ),
            (
                UnusableExtraction,
                vec![Selection, Dismiss, Selection, BeginExtraction, Extract],
                LensStage::Failed,
                None,
            ),
            (
                ObserverFailure,
                vec![
                    Selection,
                    Dismiss,
                    Selection,
                    BeginExtraction,
                    Extract,
                    Observe,
                    Degrade,
                    Transform,
                    Refresh,
                ],
                LensStage::Transforming,
                None,
            ),
            (
                TransformFailure,
                vec![
                    Selection,
                    Dismiss,
                    Selection,
                    BeginExtraction,
                    Extract,
                    Observe,
                    Transform,
                ],
                LensStage::Ready,
                Some("transform failed"),
            ),
        ] {
            let host = Host::new(scenario);
            let result = confirm_targets(&host, OPERATION).await;
            assert_eq!(
                result.as_ref().err().map(String::as_str),
                expected_error,
                "{scenario:?}"
            );
            let script = host.script.lock().unwrap();
            assert_eq!(script.effects, expected_effects, "{scenario:?}");
            assert_eq!(script.state.stage, expected_stage, "{scenario:?}");
            let superseded = matches!(
                scenario,
                SupersededDuringDismiss | SupersededDuringExtraction
            );
            assert_eq!(
                script.state.operation_id,
                Some(if superseded { REPLACEMENT } else { OPERATION }),
                "{scenario:?}"
            );
            if superseded {
                assert!(script.state.target_set.is_none());
                assert!(script.state.live.is_none());
            }
            if scenario == ObserverFailure {
                let live = script.state.live.as_ref().unwrap();
                assert_eq!(live.health, LensSourceHealth::Unavailable);
                assert_eq!(live.freshness, LensFreshness::Unverified);
                assert_eq!(live.error.as_deref(), Some("observer failed"));
            }
            if let Ok(returned) = result {
                assert_eq!(
                    serde_json::to_value(returned).unwrap(),
                    serde_json::to_value(&script.state).unwrap()
                );
            }
        }
    }

    #[tokio::test]
    async fn invalid_target_set_is_rejected_before_dismissing_preview() {
        let host = Host::new(Scenario::EmptySelection);
        assert!(confirm_targets(&host, OPERATION).await.is_err());
        let script = host.script.lock().unwrap();
        assert_eq!(script.effects, [Effect::Selection]);
        assert_eq!(script.state.stage, LensStage::Selecting);
    }
}
