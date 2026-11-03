//! Native composition supplies effects to the same Runtime-generic common shell.
mod presentation;

use super::platform::{Presentation, Services};
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    super::run_with_runtime(
        tauri::Builder::default(),
        macos_services(),
        Presentation(Arc::new(presentation::MacOsPresentation)),
    );
}

fn macos_services() -> Services {
    let native = Arc::new(adapter_platform_macos::MacOsPlatform);
    Services {
        selection: native.clone(),
        accessibility: native.clone(),
        capture: native.clone(),
        observation: native.clone(),
        trust: native,
    }
}

pub fn configure_activation<R: tauri::Runtime>(app: &mut tauri::App<R>, foreground: bool) {
    app.set_activation_policy(if foreground {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    });
}

#[allow(
    dead_code,
    reason = "Stable diagnostic composition entry; release adapter hooks are no-ops and only debug composition installs these effects"
)]
pub fn source_validation_hooks() -> crate::platform::SourceValidationHooks {
    use crate::platform::{SourceValidationCapture, SourceValidationHooks};
    use adapter_platform_macos::SourceValidationCapture as NativeCapture;
    SourceValidationHooks {
        configure: |action| {
            adapter_platform_macos::enable_source_validation_metadata();
            adapter_platform_macos::configure_source_validation_capture(action.map(|action| {
                match action {
                    SourceValidationCapture::HoldBeforeStop => NativeCapture::HoldBeforeStop,
                    SourceValidationCapture::ReportInvalid => NativeCapture::InvalidAfterStop,
                    SourceValidationCapture::ReportCleanupUnconfirmed => {
                        NativeCapture::CleanupUnconfirmedAfterStop
                    }
                }
            }));
        },
        arm_after_watching: adapter_platform_macos::arm_source_validation_capture,
    }
}
