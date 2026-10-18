#![forbid(unsafe_code)]

use crate::{accessibility, MacOsPlatform};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    authority::{TargetAuthority, TargetReceipt},
    model::{Bounds, ExtractionResult, SelectedWindow, WindowIdentity, WindowObservableFacts},
    observation::{Observation, ObservationRequest, ObservationSession},
    selection::{RegisteredWindow, TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, PlatformError, PlatformFuture,
};
use screencapturekit::{content_sharing_picker::*, shareable_content::SCWindow};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Every AppKit effect starts and stops on the process main thread. No raw handles
/// or borrowed callbacks cross this boundary.
pub(crate) fn on_main<T: Send>(action: impl FnOnce() -> T + Send) -> T {
    if objc2::MainThreadMarker::new().is_some() {
        action()
    } else {
        let mut result = None;
        dispatch2::DispatchQueue::main().exec_sync(|| result = Some(action()));
        result.expect("synchronous main-thread action completed")
    }
}

pub(crate) struct RetainedSource {
    pub(crate) selected: SelectedWindow,
    pub(crate) screen_window: SCWindow,
}

struct PickerSession {
    operation_id: Uuid,
    ticket: Uuid,
    sender: oneshot::Sender<Result<WindowPickerReply, PlatformError>>,
    subscription: Option<SCPickerSubscription>,
}

#[derive(Default)]
struct Registry {
    sources: HashMap<TargetAuthority, Arc<Mutex<RetainedSource>>>,
    picker: Option<PickerSession>,
}

#[cfg(debug_assertions)]
static SOURCE_VALIDATION_METADATA: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn enable_source_validation_metadata() {
    SOURCE_VALIDATION_METADATA.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeReleaseAck {
    Accepted,
    Rejected,
    NotAttempted,
}

#[cfg(debug_assertions)]
fn release_metadata(
    operation_id: Uuid,
    before: usize,
    after: usize,
    native_ffi_ack: NativeReleaseAck,
) {
    if SOURCE_VALIDATION_METADATA.load(std::sync::atomic::Ordering::Acquire) {
        use std::io::Write;
        let _ = writeln!(
            std::io::stdout(),
            "LENS_SOURCE_LIFECYCLE_NATIVE_RELEASE={}",
            release_metadata_value(operation_id, before, after, native_ffi_ack)
        );
    }
}

#[cfg(debug_assertions)]
fn release_metadata_value(
    operation_id: Uuid,
    before: usize,
    after: usize,
    native_ffi_ack: NativeReleaseAck,
) -> serde_json::Value {
    serde_json::json!({ "operation_id": operation_id, "rust_registry_before": before,
        "rust_registry_after": after, "native_ffi_ack": native_ffi_ack,
        "native_registry_count_observed": false })
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn operation_error(message: impl std::fmt::Display) -> PlatformError {
    PlatformError::Operation(message.to_string())
}

fn lookup_receipt<T: Clone>(
    sources: &HashMap<TargetAuthority, T>,
    target: TargetAuthority,
) -> Option<T> {
    sources.get(&target).cloned()
}

#[cfg(test)]
fn has_operation<T>(sources: &HashMap<TargetAuthority, T>, operation_id: Uuid) -> bool {
    sources
        .keys()
        .any(|target| target.operation_id == operation_id)
}

fn revoke_operation<T>(sources: &mut HashMap<TargetAuthority, T>, operation_id: Uuid) {
    sources.retain(|target, _| target.operation_id != operation_id);
}

pub(crate) fn retained_source(target: TargetAuthority) -> Option<Arc<Mutex<RetainedSource>>> {
    lookup_receipt(&registry().lock().expect("source registry").sources, target)
}

fn resolved_source(target: TargetAuthority) -> Result<Arc<Mutex<RetainedSource>>, PlatformError> {
    retained_source(target)
        .ok_or_else(|| operation_error("The exact operation-scoped target receipt is unavailable."))
}

pub(crate) fn current_capture_source(
    target: TargetAuthority,
) -> Result<(SCWindow, Bounds), PlatformError> {
    // Both registries share main-thread registration/release. Snapshot identity and
    // geometry in one transaction so a released/reselected key cannot mix sources.
    on_main(move || {
        let source = resolved_source(target)?;
        let window_id = source
            .lock()
            .expect("retained source")
            .selected
            .identity
            .window_id;
        let window = source
            .lock()
            .expect("retained source")
            .screen_window
            .clone();
        let bounds = match accessibility::current_bounds(target.operation_id, window_id)? {
            Some(bounds) => bounds,
            None => {
                // The existing screenshot fallback precedes Accessibility permission.
                // Once AX was promoted, the native result must be current or an error.
                let frame = window.frame();
                Bounds {
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                }
            }
        };
        Ok((window, bounds))
    })
}

fn picker_ticket_matches(
    pending: Option<&PickerSession>,
    operation_id: Uuid,
    ticket: Uuid,
) -> bool {
    pending.is_some_and(|pending| pending.operation_id == operation_id && pending.ticket == ticket)
}

fn take_picker(operation_id: Uuid, ticket: Uuid) -> Option<PickerSession> {
    let mut registry = registry().lock().expect("source registry");
    if picker_ticket_matches(registry.picker.as_ref(), operation_id, ticket) {
        registry.picker.take()
    } else {
        None
    }
}

fn finish_picker(
    operation_id: Uuid,
    ticket: Uuid,
    result: Result<WindowPickerReply, PlatformError>,
) {
    if let Some(session) = take_picker(operation_id, ticket) {
        drop(session.subscription);
        SCContentSharingPicker::deactivate();
        let _ = session.sender.send(result);
    }
}

fn selected_window(window: &SCWindow) -> Result<SelectedWindow, PlatformError> {
    let app = window
        .owning_application()
        .ok_or_else(|| operation_error("The selected window has no owning application."))?;
    let frame = window.frame();
    Ok(SelectedWindow {
        identity: WindowIdentity {
            window_id: window.window_id(),
            pid: app.process_id(),
            bundle_id: app.bundle_identifier(),
        },
        facts: WindowObservableFacts {
            title: window.title().unwrap_or_default(),
            application_name: app.application_name(),
            frame: Bounds {
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.size.width,
                height: frame.size.height,
            },
        },
    })
}

fn accept_selection(
    operation_id: Uuid,
    window: SCWindow,
) -> Result<WindowPickerReply, PlatformError> {
    let selected = selected_window(&window)?;
    if selected.identity.window_id == 0 {
        return Err(operation_error(
            "The selected window has no window identity.",
        ));
    }
    let mut registry = registry().lock().expect("source registry");
    // Multiple distinct windows may belong to an operation. Re-picking an
    // existing native ID must reuse its exact retained object or fail closed.
    if let Some((target, source)) = registry.sources.iter().find(|(target, source)| {
        target.operation_id == operation_id
            && source
                .lock()
                .expect("retained source")
                .selected
                .identity
                .window_id
                == selected.identity.window_id
    }) {
        let source = source.lock().expect("retained source");
        if source.screen_window != window {
            return Err(operation_error(
                "A different retained window already owns this operation/window identity.",
            ));
        }
        return Ok(WindowPickerReply::Selected {
            windows: vec![RegisteredWindow {
                target: *target,
                window: source.selected.clone(),
            }],
        });
    }
    let target = TargetAuthority {
        operation_id,
        receipt: TargetReceipt::try_from(Uuid::new_v4()).expect("generated v4 UUID is non-nil"),
    };
    accessibility::register_target(operation_id, &selected)?;
    registry.sources.insert(
        target,
        Arc::new(Mutex::new(RetainedSource {
            selected: selected.clone(),
            screen_window: window,
        })),
    );
    Ok(WindowPickerReply::Selected {
        windows: vec![RegisteredWindow {
            target,
            window: selected,
        }],
    })
}

fn begin_picker(
    operation_id: Uuid,
    ticket: Uuid,
    sender: oneshot::Sender<Result<WindowPickerReply, PlatformError>>,
) -> Result<(), PlatformError> {
    {
        let mut registry = registry().lock().expect("source registry");
        if registry.picker.is_some() {
            return Err(PlatformError::PickerBusy);
        }
        registry.picker = Some(PickerSession {
            operation_id,
            ticket,
            sender,
            subscription: None,
        });
    }
    let setup = (|| {
        let mut config = SCContentSharingPickerConfiguration::new().map_err(operation_error)?;
        config.set_allowed_picker_modes(&[SCContentSharingPickerMode::SingleWindow]);
        config.set_allows_changing_selected_content(false);
        let bundle_id = objc2_foundation::NSBundle::mainBundle().bundleIdentifier();
        let bundle_ids = bundle_id.as_ref().map(|id| id.to_string());
        let exclusions = bundle_ids.as_deref().into_iter().collect::<Vec<_>>();
        config
            .set_excluded_bundle_ids(&exclusions)
            .map_err(operation_error)?;
        SCContentSharingPicker::set_default_configuration(&config).map_err(operation_error)?;
        // The pinned Swift bridge maps zero to maximumStreamCount = nil.
        SCContentSharingPicker::set_maximum_stream_count(0).map_err(operation_error)?;
        let subscription = SCContentSharingPicker::add_observer(move |event| {
            // Never synchronously dispatch from a dependency callback: queue an owned
            // event so teardown cannot wait for a callback waiting for the main queue.
            dispatch2::DispatchQueue::main().exec_async(move || {
                // An older callback must not store a target in a newer operation.
                if !picker_ticket_matches(
                    registry().lock().expect("source registry").picker.as_ref(),
                    operation_id,
                    ticket,
                ) {
                    return;
                }
                let result = match event {
                    SCPickerEvent::Updated { result, .. } => {
                        let mut windows = result.windows();
                        if windows.len() != 1 {
                            Ok(WindowPickerReply::Error {
                                message: "Select exactly one window.".into(),
                            })
                        } else {
                            accept_selection(operation_id, windows.remove(0))
                        }
                    }
                    SCPickerEvent::Cancelled { .. } => Ok(WindowPickerReply::Cancelled),
                    SCPickerEvent::Failed(message) => Ok(WindowPickerReply::Error { message }),
                };
                finish_picker(operation_id, ticket, result);
            });
        })
        .map_err(operation_error)?;
        registry()
            .lock()
            .expect("source registry")
            .picker
            .as_mut()
            .expect("admitted picker")
            .subscription = Some(subscription);
        SCContentSharingPicker::set_active(true).map_err(operation_error)?;
        SCContentSharingPicker::present_using_style(
            screencapturekit::stream::content_filter::SCShareableContentStyle::Window,
        )
        .map_err(operation_error)?;
        Ok(())
    })();
    if let Err(error) = setup {
        finish_picker(operation_id, ticket, Err(error));
    }
    Ok(())
}

struct PendingPicker {
    operation_id: Uuid,
    ticket: Uuid,
    armed: bool,
}
impl Drop for PendingPicker {
    fn drop(&mut self) {
        if self.armed {
            let (operation_id, ticket) = (self.operation_id, self.ticket);
            on_main(move || finish_picker(operation_id, ticket, Ok(WindowPickerReply::Cancelled)));
        }
    }
}

async fn present_window_picker_for_operation(
    operation_id: Uuid,
) -> Result<WindowPickerReply, PlatformError> {
    let ticket = Uuid::new_v4();
    let (sender, receiver) = oneshot::channel();
    on_main(move || begin_picker(operation_id, ticket, sender))?;
    let mut pending = PendingPicker {
        operation_id,
        ticket,
        armed: true,
    };
    let result = receiver
        .await
        .map_err(|_| PlatformError::PickerCallbackDropped)?;
    pending.armed = false;
    result
}

fn release_registered_window(target: TargetAuthority) -> Result<(), PlatformError> {
    on_main(move || {
        // Revoke lookup before teardown, including when teardown reports failure.
        let source = registry()
            .lock()
            .expect("source registry")
            .sources
            .remove(&target)
            .ok_or_else(|| operation_error("native target receipt is unavailable"))?;
        let window_id = source
            .lock()
            .expect("retained source")
            .selected
            .identity
            .window_id;
        accessibility::release_registered_window(target.operation_id, window_id)
    })
}

fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
    on_main(move || {
        let (picker_ticket, source_count) = {
            let registry = registry().lock().expect("source registry");
            (
                registry
                    .picker
                    .as_ref()
                    .filter(|p| p.operation_id == operation_id)
                    .map(|p| p.ticket),
                registry
                    .sources
                    .keys()
                    .filter(|target| target.operation_id == operation_id)
                    .count(),
            )
        };
        if let Some(ticket) = picker_ticket {
            finish_picker(operation_id, ticket, Ok(WindowPickerReply::Cancelled));
        }
        if source_count > 0 {
            revoke_operation(
                &mut registry().lock().expect("source registry").sources,
                operation_id,
            );
            let native_result = accessibility::release_window_operation(operation_id);
            #[cfg(debug_assertions)]
            {
                let remaining = registry()
                    .lock()
                    .expect("source registry")
                    .sources
                    .keys()
                    .filter(|target| target.operation_id == operation_id)
                    .count();
                release_metadata(
                    operation_id,
                    source_count,
                    remaining,
                    if native_result.is_ok() {
                        NativeReleaseAck::Accepted
                    } else {
                        NativeReleaseAck::Rejected
                    },
                );
            }
            native_result?;
        } else {
            #[cfg(debug_assertions)]
            release_metadata(operation_id, 0, 0, NativeReleaseAck::NotAttempted);
        }
        if picker_ticket.is_some() || source_count > 0 {
            Ok(())
        } else {
            Err(operation_error(
                "native selected-window operation is unavailable",
            ))
        }
    })
}

impl TargetSelection for MacOsPlatform {
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        Box::pin(present_window_picker_for_operation(operation_id))
    }
    fn release_target(&self, target: TargetAuthority) -> Result<(), PlatformError> {
        release_registered_window(target)
    }
    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError> {
        release_window_operation(operation_id)
    }
}
impl Accessibility for MacOsPlatform {
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError> {
        match target {
            ExtractionTarget::Registered(key) => on_main(move || {
                // Admission and the native effect share the registration/release actor.
                // A released and reselected key cannot substitute a different identity.
                let source = resolved_source(key.target)?;
                let identity = source
                    .lock()
                    .expect("retained source")
                    .selected
                    .identity
                    .clone();
                accessibility::extract_registered_window(key.target.operation_id, &identity, limits)
            }),
            ExtractionTarget::Legacy(window) => accessibility::extract_window(&window, limits),
        }
    }
}
impl Observation for MacOsPlatform {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        on_main(move || {
            let source = resolved_source(request.target)?;
            let window_id = source
                .lock()
                .expect("retained source")
                .selected
                .identity
                .window_id;
            accessibility::observe(request, window_id)
        })
    }
}
impl AccessibilityTrust for MacOsPlatform {
    fn inspect(&self) -> bool {
        macos_accessibility_client::accessibility::application_is_trusted()
    }
    fn request(&self) -> bool {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_picker_ticket_cannot_cancel_a_newer_operation() {
        let operation = Uuid::from_u128(1);
        let ticket = Uuid::from_u128(2);
        let (sender, _) = oneshot::channel();
        let session = PickerSession {
            operation_id: operation,
            ticket,
            sender,
            subscription: None,
        };
        assert!(picker_ticket_matches(Some(&session), operation, ticket));
        assert!(!picker_ticket_matches(
            Some(&session),
            Uuid::from_u128(3),
            ticket
        ));
        assert!(!picker_ticket_matches(
            Some(&session),
            operation,
            Uuid::from_u128(3)
        ));
        assert!(!picker_ticket_matches(None, operation, ticket));
    }

    #[test]
    fn registered_lookup_never_reauthorizes_reused_native_metadata() {
        let operation_id = Uuid::from_u128(1);
        let first = TargetAuthority {
            operation_id,
            receipt: TargetReceipt::try_from(Uuid::from_u128(2)).unwrap(),
        };
        let second = TargetAuthority {
            operation_id,
            receipt: TargetReceipt::try_from(Uuid::from_u128(3)).unwrap(),
        };
        let wrong_operation = TargetAuthority {
            operation_id: Uuid::from_u128(4),
            ..first
        };
        let mut sources = HashMap::new();
        sources.insert(first, 17_u32);
        assert!(has_operation(&sources, operation_id));
        assert_eq!(lookup_receipt(&sources, first), Some(17));
        assert_eq!(lookup_receipt(&sources, second), None);
        assert_eq!(lookup_receipt(&sources, wrong_operation), None);
        sources.remove(&first);
        sources.insert(second, 17); // Same native ID does not revive first receipt.
        assert_eq!(lookup_receipt(&sources, first), None);
        assert_eq!(lookup_receipt(&sources, second), Some(17));
        revoke_operation(&mut sources, operation_id);
        assert_eq!(lookup_receipt(&sources, second), None);
        assert!(!has_operation(&sources, operation_id));
    }

    #[test]
    #[cfg(debug_assertions)]
    fn release_validation_distinguishes_rust_counts_and_native_ack_without_claiming_native_counts()
    {
        for (ack, expected) in [
            (NativeReleaseAck::Accepted, "accepted"),
            (NativeReleaseAck::Rejected, "rejected"),
            (NativeReleaseAck::NotAttempted, "not_attempted"),
        ] {
            let record = release_metadata_value(Uuid::from_u128(1), 2, 0, ack);
            assert_eq!(record["rust_registry_before"], 2);
            assert_eq!(record["rust_registry_after"], 0);
            assert_eq!(record["native_ffi_ack"], expected);
            assert_eq!(record["native_registry_count_observed"], false);
            assert_eq!(record.as_object().unwrap().len(), 5);
        }
    }

    #[test]
    fn owned_selected_window_handles_are_sendable() {
        fn assert_send<T: Send>() {}
        assert_send::<RetainedSource>();
        assert_send::<PickerSession>();
    }
}
