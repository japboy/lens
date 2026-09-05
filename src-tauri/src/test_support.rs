//! Explicit test-only capabilities. Unexpected native work fails instead of becoming success.
use crate::{app_state::AppState, model::AppConfig, platform, store::ConfigStore};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    capture::{Capture, CaptureBatch, CaptureRequest, CaptureTarget},
    model::ExtractionResult,
    observation::{Observation, ObservationRequest, ObservationSession},
    selection::{TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, ImageCaptureLimits, PlatformError, PlatformFuture,
};
use std::sync::Arc;
use uuid::Uuid;

struct UnusedSources;
impl Accessibility for UnusedSources {
    fn extract(
        &self,
        _: ExtractionTarget,
        _: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError> {
        panic!("unexpected Accessibility extraction")
    }
}
impl Capture for UnusedSources {
    fn capture(
        &self,
        _: CaptureTarget,
        _: &[CaptureRequest],
        _: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        panic!("unexpected native image capture")
    }
}
impl TargetSelection for UnusedSources {
    fn pick(&self, _: Uuid) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        panic!("unexpected native picker")
    }
    fn release_target(&self, _: Uuid, _: u32) -> Result<(), PlatformError> {
        panic!("unexpected target release")
    }
    fn release_operation(&self, _: Uuid) -> Result<(), PlatformError> {
        panic!("unexpected operation release")
    }
}
impl Observation for UnusedSources {
    fn observe(&self, _: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        panic!("unexpected native observation")
    }
}
impl AccessibilityTrust for UnusedSources {
    fn inspect(&self) -> bool {
        false
    }
    fn request(&self) -> bool {
        panic!("unexpected permission request")
    }
}

pub(crate) fn state() -> AppState {
    let source = Arc::new(UnusedSources);
    AppState::with_config(
        platform::Services {
            selection: source.clone(),
            accessibility: source.clone(),
            capture: source.clone(),
            observation: source.clone(),
            trust: source,
        },
        ConfigStore::at_path(
            std::env::temp_dir()
                .join(format!("lens-unused-settings-{}", Uuid::new_v4()))
                .join("settings.json"),
        ),
        AppConfig::new("/fixture".into()),
    )
}

pub(crate) struct UnusedPresentation;
impl<R: tauri::Runtime> platform::WindowPresentation<R> for UnusedPresentation {
    fn present(&self, _: &tauri::WebviewWindow<R>) -> Result<(), PlatformError> {
        panic!("unexpected native presentation")
    }
    fn dismiss<'a>(&'a self, _: &'a tauri::WebviewWindow<R>) -> platform::PresentationFuture<'a> {
        panic!("unexpected native dismissal")
    }
    fn transition<'a>(
        &'a self,
        _: &'a tauri::WebviewWindow<R>,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
    ) -> platform::PresentationFuture<'a> {
        panic!("unexpected native transition")
    }
}
