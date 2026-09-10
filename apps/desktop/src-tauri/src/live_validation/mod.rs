//! Debug-only acceptance driver over the normal native selection and live pipeline.
pub(crate) mod agent;
pub(crate) mod ports;

use crate::{
    app_state::AppState,
    commands,
    model::{
        AgentSelectionStage, AgentSelectionState, LensMonitoringLifecycle, LensStage, LensState,
    },
};
use base64::prelude::*;
use ports::{Counts, Journal};
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};

pub(crate) fn launch<R: tauri::Runtime>(app: AppHandle<R>, journal: Arc<Journal>) {
    tauri::async_runtime::spawn(async move {
        let result = tokio::time::timeout(Duration::from_secs(480), run(&app, &journal)).await;
        let result = match result {
            Ok(result) => result,
            Err(_) => Err("Native live validation exceeded 480 seconds".into()),
        };
        let mut cleanup_error = None;
        if let Ok(lens) = app.state::<AppState>().lens() {
            if let Some(operation) = lens.operation_id {
                if let Err(error) = commands::stop_lens(app.clone(), operation) {
                    cleanup_error = Some(error);
                }
            }
        }
        let passed = result.is_ok() && cleanup_error.is_none();
        println!(
            "LENS_LIVE_RESULT={}",
            serde_json::json!({
                "passed": passed, "error": result.err(), "cleanup_error": cleanup_error,
                "counts": journal.snapshot(), "external_agent": false,
            })
        );
        app.exit(if passed { 0 } else { 1 });
    });
}

fn phase(name: &str) {
    println!("LENS_LIVE_PHASE={name}");
}
fn ensure(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn quiet(a: &Counts, b: &Counts) -> bool {
    a.extractions_started == b.extractions_started
        && a.captures_started == b.captures_started
        && a.prompts == b.prompts
        && a.observations_started == b.observations_started
}
fn drained(counts: &Counts) -> bool {
    counts.extractions_started == counts.extractions_completed
        && counts.captures_started == counts.captures_completed
}
async fn drain(journal: &Journal) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if drained(&journal.snapshot()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "Native calls did not drain in 30 seconds".to_string())
}
async fn hold(journal: &Journal, baseline: &Counts) -> Result<(), String> {
    let deadline = tokio::time::Instant::now()
        + usecase::observation::PERIODIC_RECONCILIATION_INTERVAL
        + Duration::from_secs(1);
    while tokio::time::Instant::now() < deadline {
        ensure(
            quiet(baseline, &journal.snapshot()),
            "New source or Agent work started while inactive",
        )?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    ensure(
        quiet(baseline, &journal.snapshot()),
        "New source or Agent work started while inactive",
    )
}
fn identity(lens: &LensState) -> Result<serde_json::Value, String> {
    let targets = lens.target_set.as_ref().ok_or("Missing target set")?;
    let context = lens.context.as_ref().ok_or("Missing context")?;
    Ok(serde_json::json!({
        "operation": lens.operation_id, "context": context.context_id,
        "selection": targets.selection_id,
        "targets": targets.targets.iter().map(|target| serde_json::json!({
            "id": target.id, "identity": target.identity,
        })).collect::<Vec<_>>(),
    }))
}
fn lifecycle(lens: &LensState, expected: LensMonitoringLifecycle) -> bool {
    lens.live
        .as_ref()
        .is_some_and(|live| live.lifecycle == expected)
}

fn retained_representation(
    before: &crate::model::LensRepresentation,
    after: &crate::model::LensRepresentation,
) -> bool {
    // An equal-projection refresh may advance provenance before Pause takes effect.
    let mut expected = before.clone();
    expected.context_revision = after.context_revision;
    after.context_revision >= before.context_revision && after == &expected
}

fn fixture_content(
    lens: &LensState,
    payloads: &[crate::lens::LensMediaPayload],
) -> Result<(), String> {
    let context = lens.context.as_ref().ok_or("Fixture context missing")?;
    ensure(
        context.sources.len() == 1,
        "Fixture must have one structured source",
    )?;
    let source = &context.sources[0];
    println!(
        "LENS_LIVE_SOURCE={}",
        serde_json::json!({"revision": context.revision, "quality": source.quality,
            "metrics": source.capture.metrics,
            "node_count": source.document.as_ref().map(|document| document.nodes.len()),
            "media_count": context.media.len()})
    );
    let document = source
        .document
        .as_ref()
        .ok_or("Fixture structured document missing")?;
    ensure(
        document.nodes.values().any(|node| {
            [&node.title, &node.value, &node.description]
                .into_iter()
                .flatten()
                .any(|text| text.contains("LENS_ROLE_CONTENT_MARKER_7B82"))
        }),
        "Fixture text marker missing from structured nodes",
    )?;
    let image = document
        .nodes
        .values()
        .find(|node| {
            node.kind == crate::lens::LensNodeKind::Image
                && [&node.title, &node.value, &node.description]
                    .into_iter()
                    .flatten()
                    .any(|text| text == "Synthetic blue and orange role validation chart")
        })
        .ok_or("Fixture structured image node missing")?;
    let attachment = context
        .media
        .iter()
        .find(|media| {
            media.target_id == source.target_id
                && media.source_node_id.as_deref() == Some(image.id.as_str())
                && media.scope == crate::lens::LensMediaScope::AccessibilityElementRegion
                && media.geometry.is_some()
                && media.desktop_geometry.is_some()
        })
        .ok_or("Fixture image has no acquired element-region geometry")?;
    let payload = payloads
        .iter()
        .find(|payload| payload.attachment_id == attachment.id && payload.uri == attachment.uri)
        .ok_or("Fixture region payload missing")?;
    ensure(
        payload.mime_type == "image/png" && attachment.mime_type == "image/png",
        "Fixture region is not PNG",
    )?;
    let bytes = BASE64_STANDARD
        .decode(&payload.data)
        .map_err(|_| "Fixture PNG base64 is invalid")?;
    ensure(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() == attachment.encoded_bytes,
        "Fixture PNG signature or byte length mismatch",
    )?;
    ensure(
        bytes.len() >= 24 && &bytes[12..16] == b"IHDR",
        "Fixture PNG header missing",
    )?;
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| "Invalid PNG width")?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| "Invalid PNG height")?);
    ensure(
        (1..=4096).contains(&width)
            && (1..=4096).contains(&height)
            && width as usize == attachment.pixel_width
            && height as usize == attachment.pixel_height,
        "Fixture PNG dimensions exceed bounded decode or mismatch descriptor",
    )?;
    let decoded =
        tauri::image::Image::from_bytes(&bytes).map_err(|_| "Fixture PNG cannot be decoded")?;
    ensure(
        decoded.width() as usize == attachment.pixel_width
            && decoded.height() as usize == attachment.pixel_height
            && decoded.width() > 0
            && decoded.height() > 0,
        "Fixture PNG dimensions mismatch",
    )
}

fn cleared(lens: &LensState) -> bool {
    lens.operation_id.is_none()
        && lens.context.is_none()
        && lens.representation.is_none()
        && lens.target_set.is_none()
}

fn revoked<R: tauri::Runtime>(app: &AppHandle<R>, uris: &[String]) -> Result<(), String> {
    ensure(
        cleared(&app.state::<AppState>().lens()?),
        "Stop retained or republished source state",
    )?;
    for uri in uris {
        ensure(
            app.state::<AppState>()
                .lens_media
                .payload_for_uri(uri)?
                .is_none(),
            "Stop retained or republished an accessible media payload",
        )?;
    }
    Ok(())
}

async fn run<R: tauri::Runtime>(app: &AppHandle<R>, journal: &Journal) -> Result<(), String> {
    // The scripted catalog must not inherit saved choices from a real Agent.
    // This changes only this validation process's snapshot, never ConfigStore.
    app.state::<AppState>()
        .runtime
        .write()
        .map_err(|_| "Validation configuration lock is poisoned".to_string())?
        .config
        .agent_preferences = Default::default();
    let configured_agent = app.state::<AppState>().config()?.agent;
    crate::app_state::publish_agent_selection(
        app,
        AgentSelectionState {
            stage: AgentSelectionStage::Selected,
            candidate: Some(configured_agent),
            operation_id: None,
            ..AgentSelectionState::default()
        },
    )?;
    phase("select_fixture");
    let selection = commands::select_lens_target(app.clone()).await?;
    ensure(
        selection.stage == LensStage::Selecting,
        "Fixture selection did not complete",
    )?;
    let operation = selection.operation_id.ok_or("Selection has no operation")?;
    phase("confirm");
    let initial = commands::confirm_lens_targets(app.clone(), operation).await?;
    println!(
        "LENS_LIVE_INITIAL={}",
        serde_json::json!({"stage": initial.stage, "error": initial.error,
            "agent": initial.agent, "has_representation": initial.representation.is_some()})
    );
    ensure(
        initial.stage == LensStage::Completed && initial.representation.is_some(),
        "Initial interpretation not published",
    )?;
    ensure(
        lifecycle(&initial, LensMonitoringLifecycle::Watching),
        "Initial monitoring not watching",
    )?;
    let authority = identity(&initial)?;
    fixture_content(
        &initial,
        &app.state::<AppState>().lens_media.payloads(operation)?,
    )?;
    ensure(
        journal.snapshot().prompts > 0 && journal.snapshot().observations_started > 0,
        "Initial native observation or ACP prompt missing",
    )?;
    phase("pause");
    let paused = commands::pause_lens(app.clone(), operation)?;
    ensure(
        lifecycle(&paused, LensMonitoringLifecycle::Paused),
        "Pause state missing",
    )?;
    ensure(
        initial
            .representation
            .as_ref()
            .zip(paused.representation.as_ref())
            .is_some_and(|(before, after)| retained_representation(before, after)),
        "Pause changed representation",
    )?;
    let pause_boundary = journal.snapshot();
    drain(journal).await?;
    let paused_counts = journal.snapshot();
    ensure(
        quiet(&pause_boundary, &paused_counts),
        "New calls began after Pause returned",
    )?;
    ensure(
        paused_counts.observations_started == paused_counts.observations_closed,
        "Pause did not close every observer",
    )?;
    phase("paused_hold");
    hold(journal, &paused_counts).await?;
    ensure(
        app.state::<AppState>().lens()?.representation == paused.representation,
        "Paused interpretation changed during the quiet interval",
    )?;
    phase("resume");
    let resumed = commands::resume_lens(app.clone(), operation)?;
    ensure(
        lifecycle(&resumed, LensMonitoringLifecycle::Watching),
        "Resume state missing",
    )?;
    ensure(
        identity(&resumed)? == authority,
        "Resume changed target authority",
    )?;
    tokio::time::timeout(Duration::from_secs(220), async {
        loop {
            let current = app.state::<AppState>().lens()?;
            ensure(
                identity(&current)? == authority,
                "Recovery changed target authority",
            )?;
            ensure(
                current.stage != LensStage::Failed && current.error.is_none(),
                "Recovery failed",
            )?;
            let counts = journal.snapshot();
            if counts.prompts > paused_counts.prompts
                && current.stage == LensStage::Completed
                && current.representation.is_some()
                && current.representation.as_ref().map(|r| r.run_id)
                    != initial.representation.as_ref().map(|r| r.run_id)
            {
                ensure(
                    counts.observations_started > paused_counts.observations_started
                        && counts.extractions_started > paused_counts.extractions_started,
                    "Resume did not reobserve and extract",
                )?;
                ensure(
                    current.context.as_ref().map(|context| context.revision)
                        > initial.context.as_ref().map(|context| context.revision),
                    "Resume did not advance context revision",
                )?;
                return Ok::<(), String>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "Recovery checkpoint did not publish within 220 seconds")??;
    phase("stop");
    let media = app.state::<AppState>().lens_media.payloads(operation)?;
    fixture_content(&app.state::<AppState>().lens()?, &media)?;
    let media_uris: Vec<String> = media.iter().map(|payload| payload.uri.clone()).collect();
    ensure(
        !media.is_empty(),
        "Synthetic fixture produced no media to revoke",
    )?;
    let before_stop = journal.snapshot();
    let stopped = commands::stop_lens(app.clone(), operation)?;
    ensure(
        stopped.operation_id.is_none()
            && stopped.context.is_none()
            && stopped.representation.is_none()
            && stopped.target_set.is_none(),
        "Stop retained source state",
    )?;
    let stop_boundary = journal.snapshot();
    for payload in media {
        ensure(
            app.state::<AppState>()
                .lens_media
                .payload_for_uri(&payload.uri)?
                .is_none(),
            "Stop retained an accessible media payload",
        )?;
    }
    drain(journal).await?;
    let stopped_counts = journal.snapshot();
    ensure(
        quiet(&stop_boundary, &stopped_counts),
        "New calls began after Stop returned",
    )?;
    ensure(
        stopped_counts.observations_started == stopped_counts.observations_closed
            && stopped_counts.releases > before_stop.releases,
        "Stop did not close observers and release targets",
    )?;
    phase("stopped_hold");
    hold(journal, &stopped_counts).await?;
    revoked(app, &media_uris)?;
    phase("complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_fixture() -> (LensState, Vec<crate::lens::LensMediaPayload>) {
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/workspace-contracts.json"
        ))
        .unwrap();
        let mut context: crate::lens::LensContext =
            serde_json::from_value(golden["context"].clone()).unwrap();
        let source = &mut context.sources[0];
        let document = source.document.as_mut().unwrap();
        let mut node = document.nodes.values().next().unwrap().clone();
        node.id = "fixture-image".into();
        node.kind = crate::lens::LensNodeKind::Image;
        node.title = Some("Synthetic blue and orange role validation chart".into());
        node.value = Some("LENS_ROLE_CONTENT_MARKER_7B82".into());
        document.nodes.insert(node.id.clone(), node);
        let descriptor = serde_json::to_value(source.capture.geometry.as_ref().unwrap()).unwrap();
        let read = descriptor["read"].clone();
        let capture = serde_json::json!({"read":read,"capture_id":uuid::Uuid::from_u128(99)});
        let original = serde_json::json!({"kind":"original_capture_pixels","capture":capture});
        let crop = serde_json::json!({"kind":"cropped_attachment_pixels","capture":capture,"attachment_id":"fixture-media"});
        let encoded = serde_json::json!({"kind":"encoded_attachment_pixels","capture":capture,"attachment_id":"fixture-media"});
        let transform = |from: serde_json::Value, to: serde_json::Value| serde_json::json!({"read":read,"source":from,"destination":to,"scale_x":1.0,"scale_y":1.0,"translate_x":0.0,"translate_y":0.0});
        let bytes = include_bytes!("../../icons/32x32.png");
        context.media = vec![serde_json::from_value(serde_json::json!({
            "id":"fixture-media","target_id":source.target_id,"uri":"lens://fixture",
            "scope":"accessibility_element_region","source_node_id":"fixture-image",
            "geometry":{"capture":capture,"attachment_id":"fixture-media","original_extent":{"width":32,"height":32},"crop":{"x":0,"y":0,"extent":{"width":32,"height":32}},"encoded_extent":{"width":32,"height":32},"original_to_crop":transform(original,crop.clone()),"crop_to_encoded":transform(crop,encoded)},
            "desktop_geometry":descriptor,"source_bounds":{"x":0.0,"y":0.0,"width":32.0,"height":32.0},"captured_bounds":{"x":0.0,"y":0.0,"width":32.0,"height":32.0},"coverage":"full_region","coordinate_space":"screen_points","mime_type":"image/png","pixel_width":32,"pixel_height":32,"encoded_bytes":bytes.len()
        })).unwrap()];
        let payloads = vec![crate::lens::LensMediaPayload {
            attachment_id: "fixture-media".into(),
            uri: "lens://fixture".into(),
            mime_type: "image/png".into(),
            data: BASE64_STANDARD.encode(bytes),
        }];
        (
            LensState {
                context: Some(context),
                ..LensState::default()
            },
            payloads,
        )
    }

    #[test]
    fn fixture_admission_requires_matching_structured_region_and_decodable_png() {
        let (lens, payloads) = synthetic_fixture();
        assert!(fixture_content(&lens, &payloads).is_ok());
        let mut fallback = lens.clone();
        fallback.context.as_mut().unwrap().media[0].scope =
            crate::lens::LensMediaScope::WindowFallback;
        assert!(fixture_content(&fallback, &payloads).is_err());
        let mut empty = lens.clone();
        empty.context.as_mut().unwrap().sources[0]
            .document
            .as_mut()
            .unwrap()
            .nodes
            .clear();
        assert!(fixture_content(&empty, &payloads).is_err());
        let mut corrupt = payloads.clone();
        corrupt[0].data = BASE64_STANDARD.encode(b"not a PNG");
        assert!(fixture_content(&lens, &corrupt).is_err());
        assert!(fixture_content(&lens, &[]).is_err());
    }

    #[test]
    fn fixture_rejects_truncated_png_after_header_admission() {
        let (mut lens, mut payloads) = synthetic_fixture();
        let mut bytes = BASE64_STANDARD.decode(&payloads[0].data).unwrap();
        // Preserve signature and IHDR dimensions, but omit image data and IEND.
        bytes.truncate(24);
        lens.context.as_mut().unwrap().media[0].encoded_bytes = bytes.len();
        payloads[0].data = BASE64_STANDARD.encode(bytes);
        assert_eq!(
            fixture_content(&lens, &payloads).unwrap_err(),
            "Fixture PNG cannot be decoded"
        );
    }
    #[test]
    fn fixture_requires_structured_context_and_stop_requires_absent_operation() {
        let mut lens = LensState::default();
        assert!(fixture_content(&lens, &[]).is_err());
        assert!(cleared(&lens));
        lens.operation_id = Some(uuid::Uuid::from_u128(1));
        assert!(!cleared(&lens));
    }
    #[test]
    fn retention_allows_only_monotonic_context_provenance() {
        let before = crate::model::LensRepresentation {
            representation_id: uuid::Uuid::from_u128(1),
            context_id: uuid::Uuid::from_u128(2),
            context_revision: 2,
            projection: usecase::live_sync::ProjectionRef::new(
                std::num::NonZeroU64::new(1).unwrap(),
                serde_json::from_str(&format!("\"{}\"", "a".repeat(64))).unwrap(),
            ),
            run_id: uuid::Uuid::from_u128(3),
            output_blocks: vec![],
        };
        let mut after = before.clone();
        after.context_revision = 3;
        assert!(retained_representation(&before, &after));
        after.context_revision = 1;
        assert!(!retained_representation(&before, &after));
        after.context_revision = 3;
        after.run_id = uuid::Uuid::from_u128(4);
        assert!(!retained_representation(&before, &after));
    }

    #[test]
    fn completion_is_not_mistaken_for_a_new_call() {
        let before = Counts {
            captures_started: 1,
            ..Counts::default()
        };
        let mut after = before.clone();
        after.captures_completed = 1;
        assert!(quiet(&before, &after));
        assert!(!drained(&before));
        assert!(drained(&after));
        after.captures_started += 1;
        assert!(!quiet(&before, &after));
    }
}
