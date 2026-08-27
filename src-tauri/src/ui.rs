use crate::{
    commands,
    model::{
        AgentKind, AgentSelectionState, AppConfig, Bounds, LensStage, LensState, SelectedWindow,
    },
    platform,
};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::{config::WindowEffectsConfig, WindowEffect, WindowEffectState},
    App, AppHandle, LogicalPosition, LogicalSize, Manager, Position, Size, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const SETTINGS_LABEL: &str = "settings";
const SETTINGS_PREFERRED_WIDTH: f64 = 720.0;
const SETTINGS_PREFERRED_HEIGHT: f64 = 800.0;
const SETTINGS_MINIMUM_WIDTH: f64 = 420.0;
const SETTINGS_MINIMUM_HEIGHT: f64 = 360.0;
const SETTINGS_WORK_AREA_HORIZONTAL_INSET: f64 = 32.0;
const SETTINGS_WORK_AREA_VERTICAL_INSET: f64 = 48.0;
const OVERLAY_LABEL: &str = "lens-overlay";
const OVERLAY_PARENT_RATIO: f64 = 0.8;
const OVERLAY_OBSERVER_RECONCILIATION_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(2);
const OVERLAY_POLLING_FALLBACK_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(250);
const OVERLAY_TRACKING_MAX_MISSES: u8 = 8;
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
struct OverlayGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SettingsWindowSize {
    width: f64,
    height: f64,
}

impl SettingsWindowSize {
    const PREFERRED: Self = Self {
        width: SETTINGS_PREFERRED_WIDTH,
        height: SETTINGS_PREFERRED_HEIGHT,
    };

    fn from_work_area(work_area: LogicalSize<f64>) -> Self {
        let width = if work_area.width.is_finite() {
            (work_area.width - SETTINGS_WORK_AREA_HORIZONTAL_INSET).max(SETTINGS_MINIMUM_WIDTH)
        } else {
            SETTINGS_PREFERRED_WIDTH
        };
        let height = if work_area.height.is_finite() {
            (work_area.height - SETTINGS_WORK_AREA_VERTICAL_INSET).max(SETTINGS_MINIMUM_HEIGHT)
        } else {
            SETTINGS_PREFERRED_HEIGHT
        };
        Self {
            width: width.min(SETTINGS_PREFERRED_WIDTH),
            height: height.min(SETTINGS_PREFERRED_HEIGHT),
        }
    }
}

impl OverlayGeometry {
    fn from_parent(frame: Bounds) -> Option<Self> {
        if !frame.x.is_finite()
            || !frame.y.is_finite()
            || !frame.width.is_finite()
            || !frame.height.is_finite()
            || frame.width <= 0.0
            || frame.height <= 0.0
        {
            return None;
        }
        let width = frame.width * OVERLAY_PARENT_RATIO;
        let height = frame.height * OVERLAY_PARENT_RATIO;
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
            SettingsWindowSize::from_work_area(
                monitor.work_area().size.to_logical(monitor.scale_factor()),
            )
        })
        .unwrap_or(SettingsWindowSize::PREFERRED);

    WebviewWindowBuilder::new(app, SETTINGS_LABEL, webview_url(WebviewView::Settings))
        .title("PersonalLens Settings")
        .inner_size(size.width, size.height)
        .min_inner_size(SETTINGS_MINIMUM_WIDTH, SETTINGS_MINIMUM_HEIGHT)
        .resizable(true)
        .center()
        .build()?;
    Ok(())
}

pub fn show_overlay(
    app: &AppHandle,
    target: &SelectedWindow,
    operation_id: Uuid,
) -> tauri::Result<()> {
    let geometry = OverlayGeometry::from_parent(target.frame).ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "selected window has invalid bounds",
        ))
    })?;

    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        apply_overlay_geometry(&window, geometry)?;
        window.show()?;
        window.set_focus()?;
        track_parent_window(app.clone(), target.clone(), operation_id, geometry);
        return Ok(());
    }

    WebviewWindowBuilder::new(app, OVERLAY_LABEL, webview_url(WebviewView::Overlay))
        .title("PersonalLens")
        .inner_size(geometry.width, geometry.height)
        .position(geometry.x, geometry.y)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .effects(WindowEffectsConfig {
            effects: vec![WindowEffect::UnderWindowBackground],
            state: Some(WindowEffectState::Active),
            radius: Some(12.0),
            color: None,
        })
        .build()?;
    track_parent_window(app.clone(), target.clone(), operation_id, geometry);
    Ok(())
}

fn apply_overlay_geometry(window: &WebviewWindow, geometry: OverlayGeometry) -> tauri::Result<()> {
    window.set_size(Size::Logical(LogicalSize::new(
        geometry.width,
        geometry.height,
    )))?;
    window.set_position(Position::Logical(LogicalPosition::new(
        geometry.x, geometry.y,
    )))
}

fn track_parent_window(
    app: AppHandle,
    target: SelectedWindow,
    operation_id: Uuid,
    initial_geometry: OverlayGeometry,
) {
    tauri::async_runtime::spawn(async move {
        let mut observer = platform::observe_window(&target).ok();
        let mut interval = tokio::time::interval(if observer.is_some() {
            OVERLAY_OBSERVER_RECONCILIATION_INTERVAL
        } else {
            OVERLAY_POLLING_FALLBACK_INTERVAL
        });
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_geometry = initial_geometry;
        let mut consecutive_misses = 0_u8;

        loop {
            let is_current_operation = app
                .state::<crate::app_state::AppState>()
                .lens()
                .map(|lens| lens.operation_id == Some(operation_id))
                .unwrap_or(false);
            if !is_current_operation {
                break;
            }
            let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
                break;
            };

            enum TrackingSignal {
                Frame(Option<Bounds>),
                ObserverClosed,
                TargetDestroyed,
            }

            let signal = if let Some(active_observer) = observer.as_mut() {
                tokio::select! {
                    event = active_observer.recv() => match event {
                        Some(platform::WindowObserverEvent::FrameChanged { frame }) => {
                            TrackingSignal::Frame(Some(frame))
                        }
                        Some(platform::WindowObserverEvent::Destroyed) => {
                            TrackingSignal::TargetDestroyed
                        }
                        None => TrackingSignal::ObserverClosed,
                    },
                    _ = interval.tick() => {
                        TrackingSignal::Frame(read_current_parent_frame(target.window_id).await)
                    }
                }
            } else {
                interval.tick().await;
                TrackingSignal::Frame(read_current_parent_frame(target.window_id).await)
            };

            let frame = match signal {
                TrackingSignal::Frame(frame) => frame,
                TrackingSignal::TargetDestroyed => break,
                TrackingSignal::ObserverClosed => {
                    observer = None;
                    interval = tokio::time::interval(OVERLAY_POLLING_FALLBACK_INTERVAL);
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    continue;
                }
            };
            let next_geometry = frame.and_then(OverlayGeometry::from_parent);
            let Some(next_geometry) = next_geometry else {
                consecutive_misses = consecutive_misses.saturating_add(1);
                if consecutive_misses >= OVERLAY_TRACKING_MAX_MISSES {
                    break;
                }
                continue;
            };

            consecutive_misses = 0;
            if next_geometry != last_geometry {
                if apply_overlay_geometry(&window, next_geometry).is_err() {
                    break;
                }
                last_geometry = next_geometry;
            }
        }
    });
}

async fn read_current_parent_frame(window_id: u32) -> Option<Bounds> {
    match tauri::async_runtime::spawn_blocking(move || platform::current_window_frame(window_id))
        .await
    {
        Ok(Ok(frame)) => frame,
        Ok(Err(_)) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentSelectionStage;
    use std::path::PathBuf;

    #[test]
    fn settings_size_prefers_content_height_and_is_capped_by_hd_work_area() {
        assert_eq!(
            SettingsWindowSize::from_work_area(LogicalSize::new(1920.0, 1050.0)),
            SettingsWindowSize::PREFERRED
        );
        assert_eq!(
            SettingsWindowSize::from_work_area(LogicalSize::new(1280.0, 696.0)),
            SettingsWindowSize {
                width: 720.0,
                height: 648.0,
            }
        );
    }

    #[test]
    fn settings_size_preserves_its_explicit_minimum_on_a_smaller_work_area() {
        assert_eq!(
            SettingsWindowSize::from_work_area(LogicalSize::new(400.0, 300.0)),
            SettingsWindowSize {
                width: SETTINGS_MINIMUM_WIDTH,
                height: SETTINGS_MINIMUM_HEIGHT,
            }
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
    fn overlay_geometry_is_eighty_percent_and_centered_in_parent_coordinates() {
        let geometry = OverlayGeometry::from_parent(Bounds {
            x: -1440.0,
            y: 120.0,
            width: 1000.0,
            height: 700.0,
        })
        .expect("valid parent geometry");

        assert_eq!(
            geometry,
            OverlayGeometry {
                x: -1340.0,
                y: 190.0,
                width: 800.0,
                height: 560.0,
            }
        );
    }

    #[test]
    fn overlay_geometry_rejects_non_finite_or_empty_parent_bounds() {
        assert!(OverlayGeometry::from_parent(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 700.0,
        })
        .is_none());
        assert!(OverlayGeometry::from_parent(Bounds {
            x: f64::NAN,
            y: 0.0,
            width: 1000.0,
            height: 700.0,
        })
        .is_none());
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
