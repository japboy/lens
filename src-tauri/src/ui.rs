use crate::{
    commands,
    model::{
        AgentKind, AgentSelectionState, AppConfig, Bounds, LensStage, LensState, SelectedWindow,
    },
};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::{config::WindowEffectsConfig, WindowEffect, WindowEffectState},
    App, AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;

const SETTINGS_LABEL: &str = "settings";
const LENS_WINDOW_LABEL: &str = "lens-overlay";
const LENS_WINDOW_PARENT_RATIO: f64 = 0.8;
const LENS_WINDOW_CORNER_RADIUS: f64 = 12.0;
const LENS_WINDOW_EFFECT: WindowEffect = WindowEffect::HudWindow;
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-icon-template@2x.png");
const DESKTOP_PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    panic!("PersonalLens requires an explicit desktop platform presentation state")
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebviewView {
    Settings,
    Overlay,
}

impl WebviewView {
    const fn as_query_value(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Overlay => "overlay",
        }
    }
}

fn webview_url(view: WebviewView) -> WebviewUrl {
    WebviewUrl::App(
        format!(
            "index.html?view={}&platform={DESKTOP_PLATFORM}",
            view.as_query_value()
        )
        .into(),
    )
}

const SETTINGS_WINDOW_SIZE_POLICY: WindowSizePolicy = WindowSizePolicy {
    preferred: WindowSize::new(720.0, 800.0),
    minimum: WindowSize::new(420.0, 360.0),
    work_area_inset: WindowSize::new(32.0, 48.0),
};

fn tray_icon(enabled: bool) -> tauri::Result<Image<'static>> {
    let icon = Image::from_bytes(TRAY_ICON_PNG)?;
    if enabled {
        return Ok(icon);
    }

    let mut rgba = icon.rgba().to_vec();
    let (pixels, remainder) = rgba.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for pixel in pixels {
        pixel[3] /= 2;
    }
    Ok(Image::new_owned(rgba, icon.width(), icon.height()))
}

struct TrayMenuItems {
    select_target: MenuItem<tauri::Wry>,
    agent_claude: CheckMenuItem<tauri::Wry>,
    agent_codex: CheckMenuItem<tauri::Wry>,
    working_directory: MenuItem<tauri::Wry>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct WindowSize {
    width: f64,
    height: f64,
}

impl WindowSize {
    const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowSizePolicy {
    preferred: WindowSize,
    minimum: WindowSize,
    work_area_inset: WindowSize,
}

impl WindowSizePolicy {
    fn initial_size(self, work_area: LogicalSize<f64>) -> WindowSize {
        let width = if work_area.width.is_finite() {
            (work_area.width - self.work_area_inset.width).max(self.minimum.width)
        } else {
            self.preferred.width
        };
        let height = if work_area.height.is_finite() {
            (work_area.height - self.work_area_inset.height).max(self.minimum.height)
        } else {
            self.preferred.height
        };
        WindowSize {
            width: width.min(self.preferred.width),
            height: height.min(self.preferred.height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LensWindowGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl LensWindowGeometry {
    fn from_target(frame: Bounds) -> Option<Self> {
        if !frame.x.is_finite()
            || !frame.y.is_finite()
            || !frame.width.is_finite()
            || !frame.height.is_finite()
            || frame.width <= 0.0
            || frame.height <= 0.0
        {
            return None;
        }

        let width = frame.width * LENS_WINDOW_PARENT_RATIO;
        let height = frame.height * LENS_WINDOW_PARENT_RATIO;
        Some(Self {
            x: frame.x + (frame.width - width) / 2.0,
            y: frame.y + (frame.height - height) / 2.0,
            width,
            height,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TrayMenuPresentation {
    select_target_enabled: bool,
    target_selection_active: bool,
    agent_selection_enabled: bool,
    claude_checked: bool,
    codex_checked: bool,
    working_directory_text: String,
}

impl TrayMenuPresentation {
    fn derive(agent_selection: &AgentSelectionState, config: &AppConfig, lens: &LensState) -> Self {
        let selected = agent_selection.selected_agent();
        let target_selection_active = lens.stage == LensStage::Selecting;
        Self {
            select_target_enabled: agent_selection.can_select_lens_target()
                && !target_selection_active,
            target_selection_active,
            agent_selection_enabled: matches!(
                agent_selection.stage,
                crate::model::AgentSelectionStage::Unselected
                    | crate::model::AgentSelectionStage::AuthenticationRequired
                    | crate::model::AgentSelectionStage::Selected
                    | crate::model::AgentSelectionStage::Failed
            ),
            claude_checked: selected == Some(AgentKind::Claude),
            codex_checked: selected == Some(AgentKind::Codex),
            working_directory_text: format!(
                "Working Directory: {}…",
                menu_safe_path(&config.working_directory.to_string_lossy())
            ),
        }
    }
}

pub fn install_menu_bar(app: &mut App) -> tauri::Result<()> {
    let select = MenuItem::with_id(
        app,
        "select_target",
        "Select Lens Target…",
        false,
        None::<&str>,
    )?;
    let use_claude =
        CheckMenuItem::with_id(app, "agent_claude", "Claude", true, false, None::<&str>)?;
    let use_codex = CheckMenuItem::with_id(app, "agent_codex", "Codex", true, false, None::<&str>)?;
    let agent_menu = Submenu::with_items(app, "AI Agent", true, &[&use_claude, &use_codex])?;
    let working_directory = MenuItem::with_id(
        app,
        "working_directory",
        "Working Directory…",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit PersonalLens", true, None::<&str>)?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &select,
            &separator_one,
            &agent_menu,
            &working_directory,
            &settings,
            &separator_two,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("personal-lens")
        .icon(tray_icon(false)?)
        .icon_as_template(true)
        .tooltip("PersonalLens — select and authenticate an AI Agent to enable target selection")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                select_lens_target_from_tray(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "select_target" => select_lens_target_from_tray(app),
            "agent_claude" => select_agent_from_menu(app, AgentKind::Claude),
            "agent_codex" => select_agent_from_menu(app, AgentKind::Codex),
            "working_directory" => choose_working_directory(app),
            "settings" => {
                if let Err(error) = show_settings(app) {
                    eprintln!("Unable to show Settings: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(TrayMenuItems {
        select_target: select,
        agent_claude: use_claude,
        agent_codex: use_codex,
        working_directory,
    });
    sync_tray_menu(app.handle()).map_err(|error| tauri::Error::Io(std::io::Error::other(error)))?;
    Ok(())
}

pub fn sync_tray_menu(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let snapshot = state.snapshot()?;
    let presentation =
        TrayMenuPresentation::derive(&snapshot.agent_selection, &snapshot.config, &snapshot.lens);
    let items = app.state::<TrayMenuItems>();
    items
        .select_target
        .set_enabled(presentation.select_target_enabled)
        .map_err(|error| error.to_string())?;
    items
        .agent_claude
        .set_enabled(presentation.agent_selection_enabled)
        .map_err(|error| error.to_string())?;
    items
        .agent_codex
        .set_enabled(presentation.agent_selection_enabled)
        .map_err(|error| error.to_string())?;
    items
        .agent_claude
        .set_checked(presentation.claude_checked)
        .map_err(|error| error.to_string())?;
    items
        .agent_codex
        .set_checked(presentation.codex_checked)
        .map_err(|error| error.to_string())?;
    items
        .working_directory
        .set_text(presentation.working_directory_text)
        .map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id("personal-lens")
        .ok_or_else(|| "PersonalLens tray icon is unavailable".to_string())?;
    tray.set_icon_with_as_template(
        Some(tray_icon(presentation.select_target_enabled).map_err(|error| error.to_string())?),
        true,
    )
    .map_err(|error| error.to_string())?;
    let tooltip = if presentation.target_selection_active {
        "PersonalLens — Lens Target selection is already active"
    } else if presentation.select_target_enabled {
        "PersonalLens — left-click to select a Lens Target"
    } else {
        "PersonalLens — select and authenticate an AI Agent to enable target selection"
    };
    tray.set_tooltip(Some(tooltip))
        .map_err(|error| error.to_string())
}

fn select_lens_target_from_tray(app: &AppHandle) {
    let enabled = app
        .state::<crate::app_state::AppState>()
        .snapshot()
        .map(|snapshot| {
            snapshot.agent_selection.can_select_lens_target()
                && snapshot.lens.stage != LensStage::Selecting
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = commands::select_lens_target(handle).await {
            eprintln!("PersonalLens selection failed: {error}");
        }
    });
}

fn select_agent_from_menu(app: &AppHandle, agent: AgentKind) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match commands::select_agent(handle.clone(), agent).await {
            Ok(selection) if selection.selected_agent().is_some() => {}
            Ok(_) => {
                if let Err(error) = show_settings(&handle) {
                    eprintln!("Unable to show Agent authentication in Settings: {error}");
                }
            }
            Err(error) => {
                eprintln!("Unable to select Agent: {error}");
                if let Err(settings_error) = show_settings(&handle) {
                    eprintln!("Unable to show Agent error in Settings: {settings_error}");
                }
            }
        }
    });
}

fn choose_working_directory(app: &AppHandle) {
    let current = match app
        .state::<crate::app_state::AppState>()
        .config()
        .map(|config| config.working_directory)
    {
        Ok(current) => current,
        Err(_) => {
            eprintln!("Unable to read Working Directory: config state lock is poisoned");
            return;
        }
    };
    let handle = app.clone();
    app.dialog()
        .file()
        .set_title("Choose Working Directory")
        .set_directory(current)
        .pick_folder(move |selection| {
            let Some(selection) = selection else {
                return;
            };
            let directory = match selection.into_path() {
                Ok(directory) => directory,
                Err(error) => {
                    eprintln!("Unable to read selected Working Directory: {error}");
                    return;
                }
            };
            if let Err(error) = commands::update_working_directory(&handle, directory) {
                eprintln!("Unable to update Working Directory: {error}");
            }
        });
}

fn menu_safe_path(path: &str) -> String {
    path.replace('&', "&&").replace(['\r', '\n'], " ")
}

pub fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let size = app
        .primary_monitor()?
        .map(|monitor| {
            SETTINGS_WINDOW_SIZE_POLICY
                .initial_size(monitor.work_area().size.to_logical(monitor.scale_factor()))
        })
        .unwrap_or(SETTINGS_WINDOW_SIZE_POLICY.preferred);

    WebviewWindowBuilder::new(app, SETTINGS_LABEL, webview_url(WebviewView::Settings))
        .title("PersonalLens Settings")
        .inner_size(size.width, size.height)
        .min_inner_size(
            SETTINGS_WINDOW_SIZE_POLICY.minimum.width,
            SETTINGS_WINDOW_SIZE_POLICY.minimum.height,
        )
        .resizable(true)
        .center()
        .build()?;
    Ok(())
}

pub fn show_lens_window(app: &AppHandle, target: &SelectedWindow) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LENS_WINDOW_LABEL) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let geometry = LensWindowGeometry::from_target(target.frame).ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "selected window has invalid bounds",
        ))
    })?;

    WebviewWindowBuilder::new(app, LENS_WINDOW_LABEL, webview_url(WebviewView::Overlay))
        .title("PersonalLens")
        .inner_size(geometry.width, geometry.height)
        .position(geometry.x, geometry.y)
        .decorations(false)
        .always_on_top(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .effects(WindowEffectsConfig {
            effects: vec![LENS_WINDOW_EFFECT],
            state: Some(WindowEffectState::Active),
            radius: Some(LENS_WINDOW_CORNER_RADIUS),
            color: None,
        })
        .build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentSelectionStage;
    use std::path::PathBuf;

    #[test]
    fn settings_size_prefers_content_height_and_is_capped_by_hd_work_area() {
        assert_eq!(
            SETTINGS_WINDOW_SIZE_POLICY.initial_size(LogicalSize::new(1920.0, 1050.0)),
            SETTINGS_WINDOW_SIZE_POLICY.preferred
        );
        assert_eq!(
            SETTINGS_WINDOW_SIZE_POLICY.initial_size(LogicalSize::new(1280.0, 696.0)),
            WindowSize::new(720.0, 648.0)
        );
    }

    #[test]
    fn settings_size_preserves_its_explicit_minimum_on_a_smaller_work_area() {
        assert_eq!(
            SETTINGS_WINDOW_SIZE_POLICY.initial_size(LogicalSize::new(400.0, 300.0)),
            SETTINGS_WINDOW_SIZE_POLICY.minimum
        );
    }

    #[test]
    fn webview_urls_publish_explicit_finite_presentation_state() {
        let WebviewUrl::App(settings) = webview_url(WebviewView::Settings) else {
            panic!("Settings must use an application WebView URL");
        };
        let WebviewUrl::App(overlay) = webview_url(WebviewView::Overlay) else {
            panic!("Lens must use an application WebView URL");
        };

        assert_eq!(
            settings,
            std::path::PathBuf::from(format!(
                "index.html?view=settings&platform={DESKTOP_PLATFORM}"
            ))
        );
        assert_eq!(
            overlay,
            std::path::PathBuf::from(format!(
                "index.html?view=overlay&platform={DESKTOP_PLATFORM}"
            ))
        );
        assert!(matches!(DESKTOP_PLATFORM, "macos" | "windows" | "linux"));
    }

    #[test]
    fn lens_window_geometry_is_eighty_percent_and_centered_in_target_coordinates() {
        assert_eq!(
            LensWindowGeometry::from_target(Bounds {
                x: -1440.0,
                y: 120.0,
                width: 1000.0,
                height: 700.0,
            }),
            Some(LensWindowGeometry {
                x: -1340.0,
                y: 190.0,
                width: 800.0,
                height: 560.0,
            })
        );
    }

    #[test]
    fn lens_window_uses_the_semantic_hud_material() {
        assert_eq!(LENS_WINDOW_EFFECT, WindowEffect::HudWindow);
    }

    #[test]
    fn lens_window_geometry_rejects_non_finite_or_empty_target_bounds() {
        assert_eq!(
            LensWindowGeometry::from_target(Bounds {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 700.0,
            }),
            None
        );
        assert_eq!(
            LensWindowGeometry::from_target(Bounds {
                x: f64::NAN,
                y: 0.0,
                width: 1000.0,
                height: 700.0,
            }),
            None
        );
    }

    #[test]
    fn tray_presentation_has_one_check_only_for_an_authenticated_agent() {
        let config = AppConfig {
            agent: AgentKind::Codex,
            working_directory: PathBuf::from("/Users/example/Work"),
            response_prompt: AppConfig::default().response_prompt,
        };
        let selected = AgentSelectionState {
            stage: AgentSelectionStage::Selected,
            candidate: Some(AgentKind::Codex),
            ..AgentSelectionState::default()
        };

        assert_eq!(
            TrayMenuPresentation::derive(&selected, &config, &LensState::default()),
            TrayMenuPresentation {
                select_target_enabled: true,
                target_selection_active: false,
                agent_selection_enabled: true,
                claude_checked: false,
                codex_checked: true,
                working_directory_text: "Working Directory: /Users/example/Work…".into(),
            }
        );

        for stage in [
            AgentSelectionStage::Unselected,
            AgentSelectionStage::Checking,
            AgentSelectionStage::AuthenticationRequired,
            AgentSelectionStage::Authenticating,
            AgentSelectionStage::SigningOut,
            AgentSelectionStage::Failed,
        ] {
            let unauthenticated = AgentSelectionState {
                stage,
                candidate: Some(AgentKind::Codex),
                ..AgentSelectionState::default()
            };
            let presentation =
                TrayMenuPresentation::derive(&unauthenticated, &config, &LensState::default());
            assert!(!presentation.select_target_enabled);
            assert!(!presentation.claude_checked);
            assert!(!presentation.codex_checked);
            assert_eq!(
                presentation.agent_selection_enabled,
                matches!(
                    stage,
                    AgentSelectionStage::Unselected
                        | AgentSelectionStage::AuthenticationRequired
                        | AgentSelectionStage::Failed
                )
            );
        }

        let selecting = LensState {
            stage: LensStage::Selecting,
            ..LensState::default()
        };
        let presentation = TrayMenuPresentation::derive(&selected, &config, &selecting);
        assert!(!presentation.select_target_enabled);
        assert!(presentation.target_selection_active);
    }

    #[test]
    fn working_directory_menu_text_escapes_menu_mnemonics_and_line_breaks() {
        assert_eq!(menu_safe_path("/Work/R&D\nDocs"), "/Work/R&&D Docs");
    }

    #[test]
    fn tray_icon_is_a_two_x_black_and_transparent_template() {
        let icon = tray_icon(true).expect("tray icon should be a valid PNG");

        assert_eq!((icon.width(), icon.height()), (36, 36));
        assert!(
            icon.rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[0..3] == [0, 0, 0]),
            "template image RGB channels must be black"
        );
        assert!(
            icon.rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] == 0),
            "template image must preserve a transparent background"
        );
        assert!(
            icon.rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0),
            "template image must contain visible artwork"
        );
    }

    #[test]
    fn disabled_tray_icon_halves_every_alpha_channel() {
        let enabled = tray_icon(true).expect("enabled tray icon");
        let disabled = tray_icon(false).expect("disabled tray icon");

        assert_eq!((disabled.width(), disabled.height()), (36, 36));
        assert!(enabled
            .rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(disabled.rgba().as_chunks::<4>().0.iter())
            .all(|(enabled, disabled)| {
                enabled[0..3] == disabled[0..3] && disabled[3] == enabled[3] / 2
            }));
    }
}
