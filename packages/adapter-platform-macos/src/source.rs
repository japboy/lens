#![forbid(unsafe_code)]

use crate::{accessibility, MacOsPlatform};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    model::{Bounds, ExtractionResult, SelectedWindow, WindowIdentity, WindowObservableFacts},
    observation::{Observation, ObservationRequest, ObservationSession},
    selection::{TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, PlatformError, PlatformFuture,
};
use screencapturekit::{content_sharing_picker::*, shareable_content::SCWindow};
use std::{
    collections::BTreeMap,
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
    sources: BTreeMap<(Uuid, u32), Arc<Mutex<RetainedSource>>>,
    picker: Option<PickerSession>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn operation_error(message: impl std::fmt::Display) -> PlatformError {
    PlatformError::Operation(message.to_string())
}

pub(crate) fn retained_source(
    operation_id: Uuid,
    window_id: u32,
) -> Option<Arc<Mutex<RetainedSource>>> {
    registry()
        .lock()
        .expect("source registry")
        .sources
        .get(&(operation_id, window_id))
        .cloned()
}

fn admit_identity(
    retained: Option<&WindowIdentity>,
    requested: &WindowIdentity,
) -> Result<(), PlatformError> {
    if retained.is_some_and(|identity| identity != requested) {
        return Err(operation_error(
            "The requested identity does not match the retained selected window.",
        ));
    }
    // An absent source still reaches the native unavailable/error JSON contract.
    Ok(())
}

fn admit_retained_identity(
    operation_id: Uuid,
    requested: &WindowIdentity,
) -> Result<(), PlatformError> {
    let retained = retained_source(operation_id, requested.window_id);
    let retained = retained
        .as_ref()
        .map(|source| source.lock().expect("retained source"));
    admit_identity(
        retained.as_ref().map(|source| &source.selected.identity),
        requested,
    )
}

pub(crate) fn current_capture_source(
    operation_id: Uuid,
    window_id: u32,
) -> Result<(SCWindow, Bounds), PlatformError> {
    // Both registries share main-thread registration/release. Snapshot identity and
    // geometry in one transaction so a released/reselected key cannot mix sources.
    on_main(move || {
        let source = retained_source(operation_id, window_id).ok_or_else(|| {
            operation_error("The exact operation-scoped selected window is unavailable.")
        })?;
        let window = source
            .lock()
            .expect("retained source")
            .screen_window
            .clone();
        let bounds = match accessibility::current_bounds(operation_id, window_id)? {
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
    let key = (operation_id, selected.identity.window_id);
    if let Some(existing) = registry.sources.get(&key) {
        if existing.lock().expect("retained source").screen_window != window {
            return Err(operation_error(
                "A different retained window already owns this operation/window identity.",
            ));
        }
    } else {
        accessibility::register_target(operation_id, &selected)?;
        registry.sources.insert(
            key,
            Arc::new(Mutex::new(RetainedSource {
                selected: selected.clone(),
                screen_window: window,
            })),
        );
    }
    Ok(WindowPickerReply::Selected {
        windows: vec![selected],
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

fn release_registered_window(operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
    on_main(move || {
        if retained_source(operation_id, window_id).is_none() {
            return Err(operation_error(
                "native selected-window registration is unavailable",
            ));
        }
        accessibility::release_registered_window(operation_id, window_id)?;
        registry()
            .lock()
            .expect("source registry")
            .sources
            .remove(&(operation_id, window_id));
        Ok(())
    })
}

fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
    on_main(move || {
        let (picker_ticket, has_sources) = {
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
                    .any(|(operation, _)| *operation == operation_id),
            )
        };
        if let Some(ticket) = picker_ticket {
            finish_picker(operation_id, ticket, Ok(WindowPickerReply::Cancelled));
        }
        if has_sources {
            accessibility::release_window_operation(operation_id)?;
            registry()
                .lock()
                .expect("source registry")
                .sources
                .retain(|(operation, _), _| *operation != operation_id);
        }
        if picker_ticket.is_some() || has_sources {
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
    fn release_target(&self, operation_id: Uuid, window_id: u32) -> Result<(), PlatformError> {
        release_registered_window(operation_id, window_id)
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
            ExtractionTarget::Registered {
                operation_id,
                identity,
            } => on_main(move || {
                // Admission and the native effect share the registration/release actor.
                // A released and reselected key cannot substitute a different identity.
                admit_retained_identity(operation_id, &identity)?;
                accessibility::extract_registered_window(operation_id, &identity, limits)
            }),
            ExtractionTarget::Legacy(window) => accessibility::extract_window(&window, limits),
        }
    }
}
impl Observation for MacOsPlatform {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        on_main(move || {
            admit_retained_identity(request.operation_id, &request.identity)?;
            accessibility::observe(request)
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
    fn registered_admission_rejects_every_identity_component_mismatch() {
        let identity = WindowIdentity {
            window_id: 1,
            pid: 2,
            bundle_id: "registered.app".into(),
        };
        assert!(admit_identity(None, &identity).is_ok());
        assert!(admit_identity(Some(&identity), &identity).is_ok());
        let mismatches = [
            WindowIdentity {
                window_id: 3,
                ..identity.clone()
            },
            WindowIdentity {
                pid: 3,
                ..identity.clone()
            },
            WindowIdentity {
                bundle_id: "another.app".into(),
                ..identity.clone()
            },
        ];
        for requested in mismatches {
            assert!(admit_identity(Some(&identity), &requested).is_err());
        }
    }

    #[test]
    fn owned_selected_window_handles_are_sendable() {
        fn assert_send<T: Send>() {}
        assert_send::<RetainedSource>();
        assert_send::<PickerSession>();
    }
}
