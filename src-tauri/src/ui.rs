use crate::{
    commands,
    lens::LensTargetSet,
    model::{
        AgentKind, AgentSelectionState, AppConfig, Bounds, LensStage, LensState,
        LensTargetSelection,
    },
};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::{config::WindowEffectsConfig, WindowEffect, WindowEffectState},
    App, AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;

const SETTINGS_LABEL: &str = "settings";
pub(crate) const LENS_WINDOW_LABEL: &str = "lens-overlay";
pub(crate) const TARGET_SELECTION_WINDOW_LABEL: &str = "target-selection-preview";
const LENS_WINDOW_PARENT_RATIO: f64 = 0.8;
const LENS_WINDOW_CORNER_RADIUS: f64 = 12.0;
const LENS_WINDOW_EFFECT: WindowEffect = WindowEffect::HudWindow;
const TARGET_SELECTION_WINDOW_WIDTH: f64 = 240.0;
const TARGET_SELECTION_WINDOW_TOOLBAR_HEIGHT: f64 = 58.0;
const TARGET_SELECTION_WINDOW_ITEM_HEIGHT: f64 = 144.0;
const TARGET_SELECTION_WINDOW_ITEM_GAP: f64 = 10.0;
const TARGET_SELECTION_WINDOW_MARGIN: f64 = 18.0;
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-icon-template@2x.png");
const DESKTOP_PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    panic!("Lens requires an explicit desktop platform presentation state")
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebviewView {
    Settings,
    Overlay,
    TargetSelection,
}

impl WebviewView {
    const fn as_query_value(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Overlay => "overlay",
            Self::TargetSelection => "target-selection",
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct TargetSelectionWindowGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl TargetSelectionWindowGeometry {
    fn from_anchor(
        anchor: Bounds,
        item_count: usize,
        work_areas: &[(LogicalPosition<f64>, LogicalSize<f64>)],
    ) -> Option<Self> {
        if item_count == 0
            || !anchor.x.is_finite()
            || !anchor.y.is_finite()
            || !anchor.width.is_finite()
            || !anchor.height.is_finite()
            || anchor.width <= 0.0
            || anchor.height <= 0.0
        {
            return None;
        }

        let (work_area_position, work_area_size) = work_areas
            .iter()
            .enumerate()
            .filter_map(|(index, (position, size))| {
                if !position.x.is_finite()
                    || !position.y.is_finite()
                    || !size.width.is_finite()
                    || !size.height.is_finite()
                    || size.width <= 0.0
                    || size.height <= 0.0
                {
                    return None;
                }
                let left = anchor.x.max(position.x);
                let top = anchor.y.max(position.y);
                let right = (anchor.x + anchor.width).min(position.x + size.width);
                let bottom = (anchor.y + anchor.height).min(position.y + size.height);
                let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
                Some((index, intersection, (*position, *size)))
            })
            .max_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| right.0.cmp(&left.0))
            })
            .map(|(_, _, work_area)| work_area)?;

        let natural_height = TARGET_SELECTION_WINDOW_TOOLBAR_HEIGHT
            + TARGET_SELECTION_WINDOW_ITEM_HEIGHT * item_count as f64
            + TARGET_SELECTION_WINDOW_ITEM_GAP * item_count.saturating_sub(1) as f64;
        let width = TARGET_SELECTION_WINDOW_WIDTH
            .min((work_area_size.width - TARGET_SELECTION_WINDOW_MARGIN * 2.0).max(1.0));
        let height = natural_height
            .min((work_area_size.height - TARGET_SELECTION_WINDOW_MARGIN * 2.0).max(1.0));
        Some(Self {
            x: work_area_position.x + work_area_size.width - width - TARGET_SELECTION_WINDOW_MARGIN,
            y: work_area_position.y + (work_area_size.height - height) / 2.0,
            width,
            height,
        })
    }
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
        "Select Lens Targets…",
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
    let quit = MenuItem::with_id(app, "quit", "Quit Lens", true, None::<&str>)?;
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

    TrayIconBuilder::with_id("lens")
        .icon(tray_icon(false)?)
        .icon_as_template(true)
        .tooltip("Lens — select and authenticate an AI Agent to enable target selection")
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
        .tray_by_id("lens")
        .ok_or_else(|| "Lens tray icon is unavailable".to_string())?;
    tray.set_icon_with_as_template(
        Some(tray_icon(presentation.select_target_enabled).map_err(|error| error.to_string())?),
        true,
    )
    .map_err(|error| error.to_string())?;
    let tooltip = if presentation.target_selection_active {
        "Lens — Lens Target selection is already active"
    } else if presentation.select_target_enabled {
        "Lens — left-click to select Lens Targets"
    } else {
        "Lens — select and authenticate an AI Agent to enable target selection"
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
            eprintln!("Lens selection failed: {error}");
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
        .title("Lens Settings")
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

pub fn show_lens_window(app: &AppHandle, target_set: &LensTargetSet) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LENS_WINDOW_LABEL) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let placement_target = target_set.placement_target().ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Lens target set is empty",
        ))
    })?;
    let geometry = LensWindowGeometry::from_target(placement_target.frame).ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "selected window has invalid bounds",
        ))
    })?;

    WebviewWindowBuilder::new(app, LENS_WINDOW_LABEL, webview_url(WebviewView::Overlay))
        .title("Lens")
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

pub fn show_target_selection_window(
    app: &AppHandle,
    selection: &LensTargetSelection,
) -> tauri::Result<()> {
    let anchor = selection.anchor.ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target selection has no placement anchor",
        ))
    })?;
    let work_areas = app
        .available_monitors()?
        .into_iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let work_area = monitor.work_area();
            (
                work_area.position.to_logical::<f64>(scale),
                work_area.size.to_logical::<f64>(scale),
            )
        })
        .collect::<Vec<_>>();
    let geometry =
        TargetSelectionWindowGeometry::from_anchor(anchor, selection.items.len(), &work_areas)
            .ok_or_else(|| {
                tauri::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "target selection cannot be placed in an available work area",
                ))
            })?;

    if let Some(window) = app.get_webview_window(TARGET_SELECTION_WINDOW_LABEL) {
        window.set_size(LogicalSize::new(geometry.width, geometry.height))?;
        window.set_position(LogicalPosition::new(geometry.x, geometry.y))?;
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        TARGET_SELECTION_WINDOW_LABEL,
        webview_url(WebviewView::TargetSelection),
    )
    .title("Lens Target Selection")
    .inner_size(geometry.width, geometry.height)
    .position(geometry.x, geometry.y)
    .decorations(false)
    .always_on_top(true)
    .transparent(true)
    .shadow(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .skip_taskbar(true)
    .effects(WindowEffectsConfig {
        effects: vec![LENS_WINDOW_EFFECT],
        state: Some(WindowEffectState::Active),
        radius: Some(LENS_WINDOW_CORNER_RADIUS),
        color: None,
    })
    .build()?;
    Ok(())
}

pub fn close_target_selection_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(TARGET_SELECTION_WINDOW_LABEL) {
        window.destroy()?;
    }
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
        let WebviewUrl::App(target_selection) = webview_url(WebviewView::TargetSelection) else {
            panic!("Target selection must use an application WebView URL");
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
        assert_eq!(
            target_selection,
            std::path::PathBuf::from(format!(
                "index.html?view=target-selection&platform={DESKTOP_PLATFORM}"
            ))
        );
        assert!(matches!(DESKTOP_PLATFORM, "macos" | "windows" | "linux"));
    }

    #[test]
    fn target_selection_window_is_right_centered_on_the_anchor_monitor() {
        let work_areas = [
            (
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1440.0, 900.0),
            ),
            (
                LogicalPosition::new(-1440.0, 0.0),
                LogicalSize::new(1440.0, 900.0),
            ),
        ];

        assert_eq!(
            TargetSelectionWindowGeometry::from_anchor(
                Bounds {
                    x: -1200.0,
                    y: 100.0,
                    width: 800.0,
                    height: 600.0,
                },
                2,
                &work_areas,
            ),
            Some(TargetSelectionWindowGeometry {
                x: -258.0,
                y: 272.0,
                width: 240.0,
                height: 356.0,
            })
        );
    }

    #[test]
    fn target_selection_window_rejects_empty_or_invalid_selection() {
        let work_areas = [(
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1440.0, 900.0),
        )];
        assert_eq!(
            TargetSelectionWindowGeometry::from_anchor(
                Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                },
                0,
                &work_areas,
            ),
            None
        );
        assert_eq!(
            TargetSelectionWindowGeometry::from_anchor(
                Bounds {
                    x: f64::NAN,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                },
                1,
                &work_areas,
            ),
            None
        );
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
