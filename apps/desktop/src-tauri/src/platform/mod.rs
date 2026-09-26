#[allow(unused_imports)]
pub use port_platform::observation::{
    WindowObservationEvent, WindowObservationNotification, WindowObservationReceiver,
    WindowObservationRegistration, WindowObservationStart,
};
use port_platform::{
    accessibility::Accessibility, capture::Capture, observation::Observation,
    selection::TargetSelection, trust::AccessibilityTrust,
};
pub use port_platform::{ImageCaptureLimits, PlatformError};
use std::sync::Arc;

/// One explicitly supplied instance per capability; no ambient platform selection in consumers.
#[derive(Clone)]
pub struct Services {
    pub selection: Arc<dyn TargetSelection>,
    pub accessibility: Arc<dyn Accessibility>,
    pub capture: Arc<dyn Capture>,
    pub observation: Arc<dyn Observation>,
    pub trust: Arc<dyn AccessibilityTrust>,
}

/// Opaque reference surfaces and raw semantic fill/border RGBA, each composed once.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ControlColors {
    pub control_surface: [u8; 4],
    pub window_surface: [u8; 4],
    pub button_fill: [u8; 4],
    pub button_pressed_fill: [u8; 4],
    pub separator: [u8; 4],
    pub primary_button_fill: [u8; 4],
    pub primary_button_foreground: [u8; 4],
}

/// Ephemeral shared presentation. Color failure never erases accessibility/window state.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ControlPalette {
    pub colors: Option<ControlColors>,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub window_active: bool,
}

/// Document-start seed; macOS also keeps a later, native-owned script current for reloads.
pub fn control_palette_script(palette: Option<ControlPalette>) -> String {
    let json = serde_json::to_string(&palette).expect("byte palette serialization cannot fail");
    format!(
        "if(window===window.top){{window.__LENS_CONTROL_PALETTE__={json};window.dispatchEvent(new CustomEvent('lens-control-palette',{{detail:window.__LENS_CONTROL_PALETTE__}}));}}"
    )
}

/// Tauri-owned presentation effects, separate from portable source capabilities.
pub trait WindowPresentation<R: tauri::Runtime>: Send + Sync {
    fn confirm_destructive_action(
        &self,
        app: &tauri::AppHandle<R>,
        title: &str,
        message: &str,
        confirm_label: &str,
        cancel_label: &str,
    ) -> Result<bool, PlatformError>;
    fn format_short_datetime(&self, unix_seconds: f64) -> Result<String, PlatformError>;
    fn menu_presentation(
        &self,
        app: &tauri::AppHandle<R>,
        root: &tauri::menu::Menu<R>,
        history: &tauri::menu::Submenu<R>,
        rows: Vec<port_platform::MenuPresentationItem>,
    ) -> Result<(), PlatformError>;
    fn settings_background(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<tauri::utils::config::Color, PlatformError>;
    fn control_palette(
        &self,
        _app: &tauri::AppHandle<R>,
    ) -> Result<Option<ControlPalette>, PlatformError> {
        Ok(None)
    }
    fn observe_control_palette(
        &self,
        _window: &tauri::WebviewWindow<R>,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
    fn configure_floating_window_radius(
        &self,
        _window: &tauri::WebviewWindow<R>,
        _radius: f64,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
    fn present(&self, window: &tauri::WebviewWindow<R>) -> Result<(), PlatformError>;
    fn dismiss<'a>(&'a self, window: &'a tauri::WebviewWindow<R>) -> PresentationFuture<'a>;
    fn transition<'a>(
        &'a self,
        window: &'a tauri::WebviewWindow<R>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> PresentationFuture<'a>;
}

pub type PresentationFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), PlatformError>> + Send + 'a>>;

pub struct Presentation<R: tauri::Runtime>(pub Arc<dyn WindowPresentation<R>>);

pub fn present_window_from_screen_right<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window.state::<Presentation<R>>().0.present(window)
}

pub async fn dismiss_window_to_screen_right<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window.state::<Presentation<R>>().0.dismiss(window).await
}

pub async fn transition_window_frame<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    target_x: f64,
    target_y: f64,
    target_content_width: f64,
    target_content_height: f64,
) -> Result<(), PlatformError> {
    use tauri::Manager;
    window
        .state::<Presentation<R>>()
        .0
        .transition(
            window,
            target_x,
            target_y,
            target_content_width,
            target_content_height,
        )
        .await
}

#[cfg(test)]
mod control_palette_tests {
    use super::{control_palette_script, ControlColors, ControlPalette};

    #[test]
    fn color_unavailability_preserves_independent_display_state() {
        let palette = ControlPalette {
            colors: None,
            increase_contrast: true,
            reduce_transparency: true,
            window_active: false,
        };
        let value = serde_json::to_value(palette).unwrap();
        assert!(value["colors"].is_null());
        assert_eq!(value["increase_contrast"], true);
        assert_eq!(value["reduce_transparency"], true);
        assert_eq!(value["window_active"], false);
        let script = control_palette_script(Some(palette));
        assert!(script.starts_with("if(window===window.top)"));
        assert!(script.contains(r#""colors":null"#));
        assert!(control_palette_script(None).contains("__LENS_CONTROL_PALETTE__=null"));
    }

    #[test]
    fn semantic_fill_alpha_is_preserved_in_the_nested_wire_contract() {
        let palette = ControlPalette {
            colors: Some(ControlColors {
                control_surface: [248, 248, 248, 255],
                window_surface: [255, 255, 255, 255],
                button_fill: [0, 0, 0, 20],
                button_pressed_fill: [0, 0, 0, 25],
                separator: [0, 0, 0, 25],
                primary_button_fill: [0, 114, 240, 255],
                primary_button_foreground: [255, 255, 255, 255],
            }),
            increase_contrast: false,
            reduce_transparency: false,
            window_active: true,
        };
        let value = serde_json::to_value(palette).unwrap();
        assert_eq!(
            value["colors"]["button_fill"],
            serde_json::json!([0, 0, 0, 20])
        );
        assert_eq!(value["colors"]["control_surface"][3], 255);
        assert_eq!(value["colors"]["primary_button_fill"][3], 255);
        assert_eq!(
            value["colors"]["primary_button_foreground"],
            serde_json::json!([255, 255, 255, 255])
        );
        assert!(value.get("control_surface").is_none());
    }
}
