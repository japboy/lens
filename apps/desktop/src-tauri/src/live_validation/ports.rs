//! Audit real source capabilities without replacing their results or authority.
use crate::platform;
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    capture::{Capture, CaptureBatch, CaptureRequest, CaptureTarget},
    model::ExtractionResult,
    observation::{
        Observation, ObservationEvents, ObservationRegistration, ObservationRequest,
        ObservationSession, WindowObservationEvent,
    },
    selection::{TargetSelection, WindowPickerReply},
    ExtractionLimits, ImageCaptureLimits, PlatformError, PlatformFuture,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct Counts {
    pub extractions_started: usize,
    pub extractions_completed: usize,
    pub captures_started: usize,
    pub captures_completed: usize,
    pub observations_started: usize,
    pub observations_closed: usize,
    pub releases: usize,
    pub prompts: usize,
    pub observation_events_delivered: usize,
    pub observation_events_delivered_after_close: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Journal {
    extractions_started: AtomicUsize,
    extractions_completed: AtomicUsize,
    captures_started: AtomicUsize,
    captures_completed: AtomicUsize,
    observations_started: AtomicUsize,
    observations_closed: AtomicUsize,
    releases: AtomicUsize,
    prompts: AtomicUsize,
    observation_events_delivered: AtomicUsize,
    observation_events_delivered_after_close: AtomicUsize,
}

impl Journal {
    pub fn snapshot(&self) -> Counts {
        Counts {
            extractions_started: self.extractions_started.load(Ordering::SeqCst),
            extractions_completed: self.extractions_completed.load(Ordering::SeqCst),
            captures_started: self.captures_started.load(Ordering::SeqCst),
            captures_completed: self.captures_completed.load(Ordering::SeqCst),
            observations_started: self.observations_started.load(Ordering::SeqCst),
            observations_closed: self.observations_closed.load(Ordering::SeqCst),
            releases: self.releases.load(Ordering::SeqCst),
            prompts: self.prompts.load(Ordering::SeqCst),
            observation_events_delivered: self.observation_events_delivered.load(Ordering::SeqCst),
            observation_events_delivered_after_close: self
                .observation_events_delivered_after_close
                .load(Ordering::SeqCst),
        }
    }
    pub fn record_prompt(&self) {
        self.prompts.fetch_add(1, Ordering::SeqCst);
    }
}

struct Completion<'a>(&'a AtomicUsize);
impl Drop for Completion<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Audited {
    inner: platform::Services,
    journal: Arc<Journal>,
}

fn fixture_matches(application_id: &str, title: &str) -> bool {
    application_id == "com.google.chrome.for.testing"
        && matches!(
            title,
            "Lens Platform Role Fixture" | "Lens Platform Role Fixture - Google Chrome for Testing"
        )
}

pub(crate) fn instrument(inner: platform::Services, journal: Arc<Journal>) -> platform::Services {
    let trust = inner.trust.clone();
    let audited = Arc::new(Audited { inner, journal });
    platform::Services {
        selection: audited.clone(),
        accessibility: audited.clone(),
        capture: audited.clone(),
        observation: audited,
        trust,
    }
}

impl TargetSelection for Audited {
    fn open_operation(&self, operation_id: Uuid) -> Result<(), PlatformError> {
        self.inner.selection.open_operation(operation_id)
    }
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        Box::pin(async move {
            let reply = self.inner.selection.pick(operation_id).await?;
            if let WindowPickerReply::Selected { windows } = &reply {
                println!(
                    "LENS_LIVE_SELECTION={}",
                    serde_json::json!({
                        "count": windows.len(),
                        "windows": windows.iter().take(4).map(|window| serde_json::json!({
                            "application_id": window.facts.application_id.chars().take(256).collect::<String>(),
                            "title": window.facts.title.chars().take(256).collect::<String>(),
                        })).collect::<Vec<_>>()
                    })
                );
                if windows.len() != 1
                    || !fixture_matches(&windows[0].facts.application_id, &windows[0].facts.title)
                {
                    self.release_operation(operation_id)?;
                    return Err(PlatformError::Operation(
                        "Live validation requires only the exact Lens Platform Role Fixture window"
                            .into(),
                    ));
                }
            }
            Ok(reply)
        })
    }
    fn release_target(
        &self,
        operation_id: Uuid,
        receipt: port_platform::authority::TargetReceipt,
    ) -> Result<(), PlatformError> {
        self.inner.selection.release_target(operation_id, receipt)?;
        self.journal.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError> {
        self.inner.selection.release_operation(operation_id)?;
        self.journal.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
impl Accessibility for Audited {
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError> {
        self.journal
            .extractions_started
            .fetch_add(1, Ordering::SeqCst);
        let _completion = Completion(&self.journal.extractions_completed);
        self.inner.accessibility.extract(target, limits)
    }
}
impl Capture for Audited {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        limits: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        self.journal.captures_started.fetch_add(1, Ordering::SeqCst);
        let _completion = Completion(&self.journal.captures_completed);
        self.inner.capture.capture(target, requests, limits)
    }
}
impl Observation for Audited {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        let mut session = self.inner.observation.observe(request)?;
        let closed = Arc::new(AtomicBool::new(false));
        session.events = Box::new(Events {
            inner: session.events,
            journal: self.journal.clone(),
            closed: closed.clone(),
        });
        self.journal
            .observations_started
            .fetch_add(1, Ordering::SeqCst);
        session.registration = Box::new(Registration {
            inner: session.registration,
            journal: self.journal.clone(),
            closed: false,
            receiver_closed: closed,
        });
        Ok(session)
    }
}
struct Registration {
    inner: Box<dyn ObservationRegistration>,
    journal: Arc<Journal>,
    closed: bool,
    receiver_closed: Arc<AtomicBool>,
}
impl ObservationRegistration for Registration {
    fn close(&mut self) -> Result<(), PlatformError> {
        if !self.closed {
            self.inner.close()?;
            self.closed = true;
            self.receiver_closed.store(true, Ordering::SeqCst);
            self.journal
                .observations_closed
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
// Count only events delivered by the port receiver. This neither observes every
// native callback nor changes production admission of queued, stale events.
struct Events {
    inner: Box<dyn ObservationEvents>,
    journal: Arc<Journal>,
    closed: Arc<AtomicBool>,
}
impl ObservationEvents for Events {
    fn poll_next(
        &mut self,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<WindowObservationEvent>> {
        let event = self.inner.poll_next(context);
        if matches!(&event, std::task::Poll::Ready(Some(_))) {
            self.journal
                .observation_events_delivered
                .fetch_add(1, Ordering::SeqCst);
            if self.closed.load(Ordering::SeqCst) {
                self.journal
                    .observation_events_delivered_after_close
                    .fetch_add(1, Ordering::SeqCst);
            }
        }
        event
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Live validation observer cleanup failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiver_audit_counts_delivered_late_events_without_filtering_them() {
        struct Receiver(Option<WindowObservationEvent>);
        impl ObservationEvents for Receiver {
            fn poll_next(
                &mut self,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<WindowObservationEvent>> {
                std::task::Poll::Ready(self.0.take())
            }
        }
        let event: WindowObservationEvent = serde_json::from_value(serde_json::json!({
            "operation_id": Uuid::from_u128(1), "context_id": Uuid::from_u128(2),
            "source_registration_id": Uuid::from_u128(3), "observer_epoch": 1,
            "receipt": Uuid::from_u128(4), "notification": "window_moved"
        }))
        .unwrap();
        for closed in [false, true] {
            let journal = Arc::new(Journal::default());
            let mut receiver = Events {
                inner: Box::new(Receiver(Some(event.clone()))),
                journal: journal.clone(),
                closed: Arc::new(AtomicBool::new(closed)),
            };
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert_eq!(
                receiver.poll_next(&mut context),
                std::task::Poll::Ready(Some(event.clone()))
            );
            assert_eq!(
                receiver.poll_next(&mut context),
                std::task::Poll::Ready(None)
            );
            assert_eq!(journal.snapshot().observation_events_delivered, 1);
            assert_eq!(
                journal.snapshot().observation_events_delivered_after_close,
                usize::from(closed)
            );
        }
    }

    #[test]
    fn fixture_admission_accepts_only_exact_observed_title_forms() {
        let bundle = "com.google.chrome.for.testing";
        for title in [
            "Lens Platform Role Fixture",
            "Lens Platform Role Fixture - Google Chrome for Testing",
        ] {
            assert!(fixture_matches(bundle, title));
            assert!(!fixture_matches("com.google.Chrome", title));
        }
        for title in [
            "",
            "ChatGPT",
            "Lens Platform Role Fixture extra",
            "Other Lens Platform Role Fixture",
            "lens platform role fixture",
        ] {
            assert!(!fixture_matches(bundle, title));
        }
    }

    struct NativeRegistration(Arc<AtomicUsize>);
    impl ObservationRegistration for NativeRegistration {
        fn close(&mut self) -> Result<(), PlatformError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn explicit_close_and_drop_count_native_close_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let journal = Arc::new(Journal::default());
        let mut registration = Registration {
            inner: Box::new(NativeRegistration(calls.clone())),
            journal: journal.clone(),
            closed: false,
            receiver_closed: Arc::new(AtomicBool::new(false)),
        };
        registration.close().unwrap();
        registration.close().unwrap();
        drop(registration);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(journal.snapshot().observations_closed, 1);
    }

    #[test]
    fn completion_guard_records_early_error_return() {
        let completed = AtomicUsize::new(0);
        fn fail(completed: &AtomicUsize) -> Result<(), ()> {
            let _completion = Completion(completed);
            Err(())
        }
        let result = fail(&completed);
        assert!(result.is_err());
        assert_eq!(completed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_native_close_never_counts_as_closed() {
        struct FailingRegistration;
        impl ObservationRegistration for FailingRegistration {
            fn close(&mut self) -> Result<(), PlatformError> {
                Err(PlatformError::Operation("native close failed".into()))
            }
        }
        let journal = Arc::new(Journal::default());
        let mut registration = Registration {
            inner: Box::new(FailingRegistration),
            journal: journal.clone(),
            closed: false,
            receiver_closed: Arc::new(AtomicBool::new(false)),
        };
        assert!(registration.close().is_err());
        drop(registration);
        assert_eq!(journal.snapshot().observations_closed, 0);
    }
}
