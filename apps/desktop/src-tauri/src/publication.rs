//! Window-scoped presentation and identity-authorized immutable content.
//! Canonical state remains complete; large bodies never enter state events.
use crate::{app_state::AppState, model::*};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// Capacity-one presentation marker; the actor keeps its complete ordered candidate.
/// Missed deadlines delay the next deadline instead of replaying a burst of ticks.
pub(crate) struct AgentProgress {
    interval: tokio::time::Interval,
    pending: Option<bool>,
}
impl AgentProgress {
    pub fn new() -> Self {
        let period = std::time::Duration::from_millis(100);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self {
            interval,
            pending: None,
        }
    }
    pub fn record(&mut self, output_changed: bool) {
        self.pending = Some(self.pending.unwrap_or(false) || output_changed);
    }

    pub fn take_pending(&mut self) -> Option<bool> {
        self.pending.take()
    }
    pub async fn tick(&mut self) -> Option<bool> {
        self.interval.tick().await;
        self.pending.take()
    }
}

#[derive(Clone, Serialize, PartialEq)]
pub struct SourceMetadata {
    pub has_input: bool,
    pub quality: Option<ExtractionQuality>,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct WindowSnapshot {
    #[serde(flatten)]
    pub snapshot: AppSnapshot,
    pub source_ref: Option<String>,
    pub source_metadata: Option<SourceMetadata>,
    pub output_ref: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct LensSourceContent {
    pub source_ref: String,
    pub context: Option<Arc<crate::lens::LensContext>>,
    pub input: Option<Arc<crate::lens::LensInput>>,
}
#[derive(Clone, Serialize)]
pub struct LensOutputContent {
    pub output_ref: String,
    pub output_blocks: Arc<Vec<serde_json::Value>>,
    pub representation: Option<Arc<serde_json::Value>>,
}
#[derive(Clone, Serialize)]
pub struct LensImageContent {
    pub image_ref: String,
    pub data: Arc<String>,
}
struct OutputSlot {
    operation_id: Option<Uuid>,
    blocks: Arc<Vec<LensOutputBlock>>,
    representation_id: Option<Uuid>,
    content: LensOutputContent,
    images: BTreeMap<String, Arc<String>>,
}
#[derive(Default)]
struct PublicationState {
    revision: u32,
    output: Option<OutputSlot>,
    windows: BTreeMap<String, WindowSnapshot>,
}
#[derive(Default)]
pub struct PublicationStore(Mutex<PublicationState>);
fn lock_error() -> String {
    "presentation publication lock is poisoned".into()
}
fn source_ref(lens: &LensState) -> Option<String> {
    let operation = lens.operation_id?;
    let context = lens.context.as_ref()?;
    Some(format!(
        "{operation}:{}:{}",
        context.context_id, context.revision
    ))
}
fn same_output(slot: &OutputSlot, lens: &LensState) -> bool {
    slot.operation_id == lens.operation_id
        && Arc::ptr_eq(&slot.blocks, &lens.output_blocks)
        && slot.representation_id == lens.representation.as_ref().map(|r| r.representation_id)
}
fn wire_blocks(
    operation_id: Option<Uuid>,
    blocks: &[LensOutputBlock],
    images: &mut BTreeMap<String, Arc<String>>,
) -> Vec<serde_json::Value> {
    blocks.iter().map(|block| match block {
        LensOutputBlock::Image { message_id, mime_type, data, uri } => {
            let mut hash = Sha256::new();
            hash.update(mime_type.as_bytes()); hash.update([0]); hash.update(data.as_bytes());
            let digest: String = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
            let image_ref = format!("{}:{digest}", operation_id.map(|id| id.to_string()).unwrap_or_default());
            images.entry(image_ref.clone()).or_insert_with(|| Arc::new(data.clone()));
            serde_json::json!({"type":"image","message_id":message_id,"mime_type":mime_type,"data":"","uri":uri,"image_ref":image_ref})
        }
        _ => serde_json::to_value(block).expect("output block is serializable"),
    }).collect()
}
fn prepare_output(
    store: &PublicationStore,
    lens: &LensState,
) -> Result<Option<OutputSlot>, String> {
    {
        let state = store.0.lock().map_err(|_| lock_error())?;
        if state
            .output
            .as_ref()
            .is_some_and(|slot| same_output(slot, lens))
        {
            return Ok(None);
        }
    }
    if lens.output_blocks.is_empty() && lens.representation.is_none() {
        return Ok(None);
    }
    // Hash/format potentially large changed content outside every shared guard.
    let mut images = BTreeMap::new();
    let blocks = wire_blocks(lens.operation_id, &lens.output_blocks, &mut images);
    let representation = lens.representation.as_ref().map(|representation| {
        let mut metadata = representation.clone();
        metadata.output_blocks = Default::default();
        let mut value = serde_json::to_value(metadata).expect("representation is serializable");
        value["output_blocks"] = wire_blocks(
            lens.operation_id,
            &representation.output_blocks,
            &mut images,
        )
        .into();
        value
    });
    Ok(Some(OutputSlot {
        operation_id: lens.operation_id,
        blocks: Arc::clone(&lens.output_blocks),
        representation_id: lens.representation.as_ref().map(|r| r.representation_id),
        content: LensOutputContent {
            output_ref: Uuid::new_v4().to_string(),
            output_blocks: blocks.into(),
            representation: representation.map(Arc::new),
        },
        images,
    }))
}
fn project(snapshot: &AppSnapshot, label: &str, output_ref: Option<String>) -> WindowSnapshot {
    let overlay = label == "lens-overlay";
    // Construct the body-free projection before cloning. Never clone source or image bytes here.
    let source_ref = overlay.then(|| source_ref(&snapshot.lens)).flatten();
    let mut lens = if overlay {
        LensState {
            prompt_execution_revision: snapshot.lens.prompt_execution_revision,
            session_controls: snapshot.lens.session_controls.clone(),
            operation_id: snapshot.lens.operation_id,
            stage: snapshot.lens.stage,
            selection: snapshot.lens.selection.clone(),
            target_set: snapshot.lens.target_set.clone(),
            context: None,
            input: None,
            projection: snapshot.lens.projection.clone(),
            output_blocks: Default::default(),
            representation: snapshot.lens.representation.as_ref().map(|r| {
                let mut r = r.clone();
                r.output_blocks = Default::default();
                r
            }),
            response_history: snapshot.lens.response_history.clone(),
            pending_representation: snapshot.lens.pending_representation.clone(),
            live: snapshot.lens.live.clone(),
            agent: snapshot.lens.agent.clone(),
            error: snapshot.lens.error.clone(),
        }
    } else {
        LensState::default()
    };
    if !overlay {
        lens.stage = snapshot.lens.stage;
    }
    if label == crate::ui::TARGET_SELECTION_WINDOW_LABEL {
        lens.operation_id = snapshot.lens.operation_id;
        lens.selection = snapshot.lens.selection.clone();
        lens.error = snapshot.lens.error.clone();
    }
    WindowSnapshot {
        snapshot: AppSnapshot {
            revision: snapshot.revision,
            config: snapshot.config.clone(),
            agent_runtime: snapshot.agent_runtime.clone(),
            agent_selection: snapshot.agent_selection.clone(),
            lens,
        },
        source_ref,
        source_metadata: overlay.then(|| SourceMetadata {
            has_input: snapshot.lens.input.is_some(),
            quality: snapshot
                .lens
                .context
                .as_ref()
                .map(|context| context.quality),
        }),
        output_ref: overlay.then_some(output_ref).flatten(),
    }
}
fn same_projection(left: &WindowSnapshot, right: &WindowSnapshot) -> bool {
    // Exhaustive destructuring keeps new fields in the comparison contract.
    let WindowSnapshot {
        snapshot:
            AppSnapshot {
                revision: _,
                config,
                agent_runtime,
                agent_selection,
                lens,
            },
        source_ref,
        source_metadata,
        output_ref,
    } = left;
    (
        config,
        agent_runtime,
        agent_selection,
        lens,
        source_ref,
        source_metadata,
        output_ref,
    ) == (
        &right.snapshot.config,
        &right.snapshot.agent_runtime,
        &right.snapshot.agent_selection,
        &right.snapshot.lens,
        &right.source_ref,
        &right.source_metadata,
        &right.output_ref,
    )
}
pub fn publish<R: tauri::Runtime>(app: &AppHandle<R>, snapshot: AppSnapshot) -> Result<(), String> {
    let state = app.state::<AppState>();
    let labels: Vec<_> = app.webview_windows().into_keys().collect();
    let prepared = prepare_output(&state.publication, &snapshot.lens)?;
    let pending = {
        let mut publication = state.publication.0.lock().map_err(|_| lock_error())?;
        if snapshot.revision < publication.revision {
            return Ok(());
        }
        publication.revision = snapshot.revision;
        if !publication
            .output
            .as_ref()
            .is_some_and(|slot| same_output(slot, &snapshot.lens))
        {
            publication.output = prepared;
        }
        let output_ref = publication
            .output
            .as_ref()
            .map(|slot| slot.content.output_ref.clone());
        let mut pending = Vec::new();
        for label in labels {
            let wire = project(&snapshot, &label, output_ref.clone());
            if publication
                .windows
                .get(&label)
                .is_some_and(|previous| same_projection(previous, &wire))
            {
                continue;
            }
            publication.windows.insert(label.clone(), wire.clone());
            pending.push((label, wire));
        }
        pending
    };
    // Native delivery never owns an application, controls, or publication guard.
    // Each complete window projection carries the canonical revision, so late
    // delivery is rejected by its frontend independently of other windows.
    let mut first_error = None;
    for (label, wire) in pending {
        if let Err(error) = app.emit_to(&label, "window-app-state-changed", &wire) {
            let mut publication = state.publication.0.lock().map_err(|_| lock_error())?;
            if publication
                .windows
                .get(&label)
                .is_some_and(|current| current.snapshot.revision == wire.snapshot.revision)
            {
                publication.windows.remove(&label);
            }
            first_error.get_or_insert_with(|| error.to_string());
        }
    }
    first_error.map_or(Ok(()), Err)
}
pub fn window_snapshot(state: &AppState, label: &str) -> Result<WindowSnapshot, String> {
    let snapshot = state.snapshot()?;
    if label != "lens-overlay" {
        return Ok(project(&snapshot, label, None));
    }
    let prepared = prepare_output(&state.publication, &snapshot.lens)?;
    let mut publication = state.publication.0.lock().map_err(|_| lock_error())?;
    // Fetch cannot replace resources established by a newer concurrent publication.
    if snapshot.revision >= publication.revision {
        publication.revision = snapshot.revision;
        if !publication
            .output
            .as_ref()
            .is_some_and(|slot| same_output(slot, &snapshot.lens))
        {
            publication.output = prepared;
        }
    }
    let output_ref = publication
        .output
        .as_ref()
        .filter(|slot| same_output(slot, &snapshot.lens))
        .map(|slot| slot.content.output_ref.clone());
    Ok(project(&snapshot, label, output_ref))
}
pub fn source(state: &AppState, expected: &str) -> Result<LensSourceContent, String> {
    let runtime = state
        .runtime
        .read()
        .map_err(|_| "application state lock is poisoned")?;
    if source_ref(&runtime.lens).as_deref() != Some(expected) {
        return Err("Lens source was superseded".into());
    }
    Ok(LensSourceContent {
        source_ref: expected.into(),
        context: runtime.lens.context.clone(),
        input: runtime.lens.input.clone(),
    })
}
pub fn output(state: &AppState, expected: &str) -> Result<LensOutputContent, String> {
    let runtime = state
        .runtime
        .read()
        .map_err(|_| "application state lock is poisoned")?;
    let publication = state.publication.0.lock().map_err(|_| lock_error())?;
    publication
        .output
        .as_ref()
        .filter(|slot| same_output(slot, &runtime.lens) && slot.content.output_ref == expected)
        .map(|slot| slot.content.clone())
        .ok_or_else(|| "Lens output was superseded".into())
}
pub fn image(state: &AppState, expected: &str) -> Result<LensImageContent, String> {
    let runtime = state
        .runtime
        .read()
        .map_err(|_| "application state lock is poisoned")?;
    let publication = state.publication.0.lock().map_err(|_| lock_error())?;
    publication
        .output
        .as_ref()
        .filter(|slot| same_output(slot, &runtime.lens))
        .and_then(|slot| slot.images.get(expected))
        .map(|data| LensImageContent {
            image_ref: expected.into(),
            data: data.clone(),
        })
        .ok_or_else(|| "Lens image was superseded".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Listener;
    fn image_block() -> LensOutputBlock {
        LensOutputBlock::Image {
            message_id: Some("image".into()),
            mime_type: "image/png".into(),
            data: "x".repeat(2 * 1024 * 1024),
            uri: None,
        }
    }
    fn image_state() -> AppState {
        let state = crate::test_support::state();
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.lens.operation_id = Some(Uuid::from_u128(1));
        snapshot.lens.stage = LensStage::Transforming;
        snapshot.lens.output_blocks = vec![image_block()].into();
        drop(snapshot);
        state
    }
    #[test]
    fn representation_clones_share_image_storage_and_overlay_keeps_only_metadata() {
        let state = crate::test_support::state();
        let representation = crate::model::LensRepresentation {
            prompt_execution_revision: 7,
            representation_id: Uuid::from_u128(2),
            context_id: Uuid::from_u128(3),
            context_revision: 11,
            projection: serde_json::from_value(serde_json::json!({
                "revision": 5,
                "digest": "a".repeat(64),
            }))
            .unwrap(),
            run_id: Uuid::from_u128(4),
            output_blocks: vec![image_block()].into(),
        };
        let cloned = representation.clone();
        // A full representation clone must retain the same allocation, including
        // its multi-megabyte image string, before publication strips its body.
        assert!(Arc::ptr_eq(
            &cloned.output_blocks,
            &representation.output_blocks
        ));
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.lens.operation_id = Some(Uuid::from_u128(1));
            snapshot.lens.representation = Some(cloned);
        }

        let snapshot = state.snapshot().unwrap();
        assert!(Arc::ptr_eq(
            &snapshot.lens.representation.as_ref().unwrap().output_blocks,
            &representation.output_blocks
        ));
        let wire = window_snapshot(&state, "lens-overlay").unwrap();
        let metadata = wire.snapshot.lens.representation.as_ref().unwrap();
        assert_eq!(
            metadata.prompt_execution_revision,
            representation.prompt_execution_revision
        );
        assert_eq!(metadata.representation_id, representation.representation_id);
        assert_eq!(metadata.context_id, representation.context_id);
        assert_eq!(metadata.context_revision, representation.context_revision);
        assert_eq!(metadata.projection, representation.projection);
        assert_eq!(metadata.run_id, representation.run_id);
        assert!(metadata.output_blocks.is_empty());
        assert!(serde_json::to_vec(&wire).unwrap().len() < 64 * 1024);

        let content = output(&state, wire.output_ref.as_deref().unwrap()).unwrap();
        assert!(content.output_blocks.is_empty());
        let image_ref = content.representation.as_ref().unwrap()["output_blocks"][0]["image_ref"]
            .as_str()
            .unwrap();
        assert_eq!(
            image(&state, image_ref).unwrap().data.as_str(),
            "x".repeat(2 * 1024 * 1024)
        );
    }

    #[test]
    fn window_events_are_body_free_and_image_identity_survives_text_updates() {
        let state = image_state();
        let wire = window_snapshot(&state, "lens-overlay").unwrap();
        let first = output(&state, wire.output_ref.as_deref().unwrap()).unwrap();
        let image_ref = first.output_blocks[0]["image_ref"].as_str().unwrap();
        assert!(serde_json::to_vec(&wire).unwrap().len() < 64 * 1024);
        assert!(serde_json::to_vec(&first).unwrap().len() < 1024);
        assert_eq!(
            image(&state, image_ref).unwrap().data.len(),
            2 * 1024 * 1024
        );
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision += 1;
            snapshot.lens.output_blocks = vec![
                image_block(),
                LensOutputBlock::Markdown {
                    message_id: None,
                    text: "new text".into(),
                },
            ]
            .into();
        }
        let next = window_snapshot(&state, "lens-overlay").unwrap();
        assert_ne!(wire.output_ref, next.output_ref);
        assert!(output(&state, wire.output_ref.as_deref().unwrap()).is_err());
        let next_output = output(&state, next.output_ref.as_deref().unwrap()).unwrap();
        assert_eq!(next_output.output_blocks[0]["image_ref"], image_ref);
        assert_eq!(next_output.output_blocks[1]["text"], "new text");
    }
    #[test]
    fn progress_retains_resource_refs_and_other_window_projection() {
        let state = image_state();
        let initial = window_snapshot(&state, "lens-overlay").unwrap();
        let settings = window_snapshot(&state, "settings").unwrap();
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision += 1;
            snapshot.lens.pending_representation = None;
        }
        let next = window_snapshot(&state, "lens-overlay").unwrap();
        assert_eq!(initial.output_ref, next.output_ref);
        assert!(same_projection(
            &settings,
            &window_snapshot(&state, "settings").unwrap()
        ));
        assert!(settings.output_ref.is_none());
        assert!(settings.snapshot.lens.output_blocks.is_empty());
    }
    #[test]
    fn stop_and_replacement_revoke_old_resources_even_for_identical_images() {
        let state = image_state();
        let wire = window_snapshot(&state, "lens-overlay").unwrap();
        let content = output(&state, wire.output_ref.as_deref().unwrap()).unwrap();
        let old_image_ref = content.output_blocks[0]["image_ref"].as_str().unwrap();
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision += 1;
            snapshot.lens = LensState::default();
        }
        assert!(output(&state, wire.output_ref.as_deref().unwrap()).is_err());
        assert!(image(&state, old_image_ref).is_err());
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision += 1;
            snapshot.lens.operation_id = Some(Uuid::from_u128(2));
            snapshot.lens.output_blocks = vec![image_block()].into();
        }
        let next = window_snapshot(&state, "lens-overlay").unwrap();
        assert_ne!(wire.output_ref, next.output_ref);
        assert!(image(&state, old_image_ref).is_err());
    }
    #[test]
    fn late_publisher_cannot_replace_newer_resource_authority() {
        let state = image_state();
        let old = state.snapshot().unwrap();
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap();
        let state = app.state::<AppState>();
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision = 10;
            snapshot.lens = LensState::default();
        }
        publish(app.handle(), state.snapshot().unwrap()).unwrap();
        publish(app.handle(), old).unwrap();
        assert!(state.publication.0.lock().unwrap().output.is_none());
        assert_eq!(state.publication.0.lock().unwrap().revision, 10);
    }
    #[test]
    fn event_delivery_holds_no_publication_or_runtime_lock() {
        let app = tauri::test::mock_builder()
            .manage(image_state())
            .build(crate::product_context())
            .unwrap();
        tauri::WebviewWindowBuilder::new(&app, "lens-overlay", Default::default())
            .build()
            .unwrap();
        let handle = app.handle().clone();
        let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let received = observed.clone();
        app.listen_any("window-app-state-changed", move |_| {
            let state = handle.state::<AppState>();
            assert!(state.runtime.try_write().is_ok());
            assert!(state.publication.0.try_lock().is_ok());
            received.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        publish(app.handle(), app.state::<AppState>().snapshot().unwrap()).unwrap();
        assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn progress_bursts_coalesce_without_catching_up_after_a_stall() {
        let mut progress = AgentProgress::new();
        assert_eq!(progress.tick().await, None);
        for i in 0..1000 {
            progress.record(i == 40);
        }
        tokio::time::advance(std::time::Duration::from_millis(99)).await;
        tokio::select! {
            biased;
            _ = progress.tick() => panic!("progress published before its deadline"),
            _ = std::future::ready(()) => {}
        }
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert_eq!(progress.tick().await, Some(true));
        progress.record(false);
        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        assert_eq!(progress.tick().await, Some(false));
        progress.record(true);
        tokio::select! {
            biased;
            _ = progress.tick() => panic!("stalled timer replayed an immediate catch-up tick"),
            _ = std::future::ready(()) => {}
        }
        tokio::time::advance(std::time::Duration::from_millis(100)).await;
        assert_eq!(progress.tick().await, Some(true));
    }

    #[test]
    fn source_read_shares_one_atomic_context_and_input_then_rejects_replacement() {
        let state = image_state();
        let context = Arc::new(crate::lens::LensContext {
            schema_version: crate::lens::LENS_CONTEXT_SCHEMA_VERSION,
            context_id: Uuid::from_u128(7),
            revision: 4,
            sources: vec![],
            media: vec![],
            media_omissions: vec![],
            quality: crate::model::ExtractionQuality::Full,
            diagnostics: vec![],
        });
        let input = Arc::new(crate::lens::LensInput {
            schema_version: 1,
            context_id: context.context_id,
            context_revision: context.revision,
            sources: vec![],
            media: vec![],
            media_omissions: vec![],
            quality: crate::model::ExtractionQuality::Full,
        });
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.lens.context = Some(context.clone());
            snapshot.lens.input = Some(input.clone());
        }
        let wire = window_snapshot(&state, "lens-overlay").unwrap();
        assert!(wire.source_metadata.as_ref().unwrap().has_input);
        assert_eq!(
            wire.source_metadata.as_ref().unwrap().quality,
            Some(ExtractionQuality::Full)
        );
        let expected = wire.source_ref.unwrap();
        let content = source(&state, &expected).unwrap();
        assert!(Arc::ptr_eq(content.context.as_ref().unwrap(), &context));
        assert!(Arc::ptr_eq(content.input.as_ref().unwrap(), &input));
        let cloned = state.snapshot().unwrap();
        assert!(Arc::ptr_eq(cloned.lens.context.as_ref().unwrap(), &context));
        assert!(Arc::ptr_eq(cloned.lens.input.as_ref().unwrap(), &input));
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.revision += 1;
            snapshot.lens.operation_id = Some(Uuid::from_u128(9));
        }
        assert!(source(&state, &expected).is_err());
    }
}
