use crate::{
    commands,
    lens::LensTargetSet,
    model::{
        AgentKind, AgentSelectionState, AppConfig, Bounds, LensStage, LensState,
        LensTargetSelection,
    },
};
use std::sync::{Arc, Mutex};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::{config::WindowEffectsConfig, WindowEffect, WindowEffectState},
    App, AppHandle, Emitter, LogicalPosition, LogicalSize, LogicalUnit, Manager, WebviewUrl,
    WebviewWindowBuilder, WindowSizeConstraints,
};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const SETTINGS_LABEL: &str = "settings";
const SETTINGS_WINDOW_MIN_WIDTH: f64 = 715.0;
const SETTINGS_SIDEBAR_WIDTH: f64 = 188.0;
const SETTINGS_DETAIL_HORIZONTAL_PADDING: f64 = 52.0;
const SETTINGS_CONTENT_MAX_WIDTH: f64 = 680.0;
const SETTINGS_WINDOW_MAX_WIDTH: f64 =
    SETTINGS_SIDEBAR_WIDTH + SETTINGS_DETAIL_HORIZONTAL_PADDING + SETTINGS_CONTENT_MAX_WIDTH;
pub(crate) const LENS_WINDOW_LABEL: &str = "lens-overlay";
pub(crate) const TARGET_SELECTION_WINDOW_LABEL: &str = "target-selection-preview";
const LENS_SINGLE_TARGET_SIZE_RATIO: f64 = 0.8;
const LENS_MULTIPLE_TARGET_SCREEN_HEIGHT_RATIO: f64 = 0.8;
const LENS_MULTIPLE_TARGET_ASPECT_WIDTH: f64 = 10.0;
const LENS_MULTIPLE_TARGET_ASPECT_HEIGHT: f64 = 16.0;
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
    About,
    Settings,
    Overlay,
    TargetSelection,
}

impl WebviewView {
    const fn entry_path(self) -> &'static str {
        match self {
            Self::About => "about.html",
            Self::Settings => "settings.html",
            Self::Overlay => "overlay.html",
            Self::TargetSelection => "target-selection.html",
        }
    }
}

fn webview_url(view: WebviewView) -> WebviewUrl {
    WebviewUrl::App(format!("{}?platform={DESKTOP_PLATFORM}", view.entry_path()).into())
}

pub(crate) fn show_settings_recovery<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(
        app,
        crate::settings_recovery::WINDOW_LABEL,
        WebviewUrl::App(format!("settings-recovery.html?platform={DESKTOP_PLATFORM}").into()),
    )
    .title("Lens Settings Recovery")
    .inner_size(640.0, 460.0)
    .min_inner_size(480.0, 360.0)
    .build()?;
    present_window(&window)
}

/// Present a requested window consistently, whether newly created, hidden, or minimized.
fn present_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> tauri::Result<()> {
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

const SETTINGS_WINDOW_SIZE_POLICY: WindowSizePolicy = WindowSizePolicy {
    preferred: WindowSize::new(720.0, 800.0),
    minimum: WindowSize::new(SETTINGS_WINDOW_MIN_WIDTH, 360.0),
    maximum_width: SETTINGS_WINDOW_MAX_WIDTH,
    work_area_inset: WindowSize::new(32.0, 48.0),
    maximizable: true,
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

struct TrayMenuItems<R: tauri::Runtime> {
    root: Menu<R>,
    history: Submenu<R>,
    history_presentation: Mutex<String>,
    select_target: MenuItem<R>,
    agents: Submenu<R>,
    agent_presentation: Mutex<AgentMenuState>,
    working_directory: MenuItem<R>,
    prompt_presets: Submenu<R>,
    prompt_presentation: Mutex<Vec<PromptPresetMenuItem>>,
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
    maximum_width: f64,
    work_area_inset: WindowSize,
    maximizable: bool,
}

impl WindowSizePolicy {
    fn initial_size(self, work_area: LogicalSize<f64>) -> WindowSize {
        debug_assert!(self.resizable_width_range_is_valid());
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
            width: width.min(self.preferred.width).min(self.maximum_width),
            height: height.min(self.preferred.height),
        }
    }

    fn constraints(self) -> WindowSizeConstraints {
        WindowSizeConstraints {
            min_width: Some(LogicalUnit::new(self.minimum.width).into()),
            min_height: Some(LogicalUnit::new(self.minimum.height).into()),
            max_width: Some(LogicalUnit::new(self.maximum_width).into()),
            max_height: None,
        }
    }

    fn resizable_width_range_is_valid(self) -> bool {
        self.minimum.width <= self.preferred.width && self.preferred.width <= self.maximum_width
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LensWindowGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LensWindowPlacementDecision {
    Apply,
    Preserve,
}

#[derive(Debug, Default)]
struct LensWindowPlacementState {
    applied_selection_id: Option<Uuid>,
}

impl LensWindowPlacementState {
    fn decision(&self, selection_id: Uuid, window_exists: bool) -> LensWindowPlacementDecision {
        if window_exists && self.applied_selection_id == Some(selection_id) {
            LensWindowPlacementDecision::Preserve
        } else {
            LensWindowPlacementDecision::Apply
        }
    }

    fn record_applied(&mut self, selection_id: Uuid) {
        self.applied_selection_id = Some(selection_id);
    }
}

#[derive(Default)]
pub struct LensWindowPresentationState {
    placement: Mutex<LensWindowPlacementState>,
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

        let width = frame.width * LENS_SINGLE_TARGET_SIZE_RATIO;
        let height = frame.height * LENS_SINGLE_TARGET_SIZE_RATIO;
        Some(Self {
            x: frame.x + (frame.width - width) / 2.0,
            y: frame.y + (frame.height - height) / 2.0,
            width,
            height,
        })
    }

    fn from_primary_screen(position: LogicalPosition<f64>, size: LogicalSize<f64>) -> Option<Self> {
        if !position.x.is_finite()
            || !position.y.is_finite()
            || !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return None;
        }

        let height = size.height * LENS_MULTIPLE_TARGET_SCREEN_HEIGHT_RATIO;
        let width = height * LENS_MULTIPLE_TARGET_ASPECT_WIDTH / LENS_MULTIPLE_TARGET_ASPECT_HEIGHT;
        Some(Self {
            x: position.x + (size.width - width) / 2.0,
            y: position.y + (size.height - height) / 2.0,
            width,
            height,
        })
    }
}

fn lens_window_geometry<R: tauri::Runtime>(
    app: &AppHandle<R>,
    target_set: &LensTargetSet,
) -> tauri::Result<LensWindowGeometry> {
    match target_set.targets.as_slice() {
        [] => Err(tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Lens target set is empty",
        ))),
        [target] => LensWindowGeometry::from_target(target.facts.frame).ok_or_else(|| {
            tauri::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "selected window has invalid bounds",
            ))
        }),
        [_, _, ..] => primary_screen_overlay_geometry(app),
    }
}

/// Shared placement for overlays without a single source-window anchor.
fn primary_screen_overlay_geometry<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> tauri::Result<LensWindowGeometry> {
    let monitor = app.primary_monitor()?.ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "primary monitor is unavailable",
        ))
    })?;
    let scale_factor = monitor.scale_factor();
    let position = monitor.position().to_logical::<f64>(scale_factor);
    let size = monitor.size().to_logical::<f64>(scale_factor);
    LensWindowGeometry::from_primary_screen(position, size).ok_or_else(|| {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "primary monitor has invalid bounds",
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptPresetMenuItem {
    id: String,
    name: String,
    checked: bool,
}

fn prompt_preset_menu(config: &AppConfig) -> Vec<PromptPresetMenuItem> {
    config
        .prompt_presets
        .presets
        .iter()
        .map(|preset| PromptPresetMenuItem {
            id: preset.id.clone(),
            name: preset.name.replace('&', "&&"),
            checked: preset.id == config.prompt_presets.selected_id,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayMenuPresentation {
    select_target_enabled: bool,
    target_selection_active: bool,
    live_lens_active: bool,
    agent_selection_enabled: bool,
    agent_verification_required: bool,
    claude_checked: bool,
    codex_checked: bool,
    selected_agent: Option<AgentKind>,
    working_directory_text: String,
}

/// Desktop-owned tray output; derivation remains shared application presentation policy.
pub(crate) trait TrayOutput<R: tauri::Runtime>: Send + Sync {
    fn apply(&self, app: &AppHandle<R>, presentation: TrayMenuPresentation) -> Result<(), String>;
}

pub(crate) struct TrayPresentation<R: tauri::Runtime>(pub Arc<dyn TrayOutput<R>>);

pub(crate) struct NativeTrayOutput;

impl TrayMenuPresentation {
    fn derive(agent_selection: &AgentSelectionState, config: &AppConfig, lens: &LensState) -> Self {
        let selected =
            if agent_selection.stage == crate::model::AgentSelectionStage::HistorySelected {
                agent_selection.candidate
            } else {
                agent_selection.selected_agent()
            };
        let target_selection_active = lens.stage == LensStage::Selecting;
        let live_lens_active = lens.live.is_some();
        Self {
            select_target_enabled: agent_selection.can_select_lens_target()
                && !target_selection_active
                && !live_lens_active,
            target_selection_active,
            live_lens_active,
            agent_selection_enabled: matches!(
                agent_selection.stage,
                crate::model::AgentSelectionStage::Unselected
                    | crate::model::AgentSelectionStage::HistorySelected
                    | crate::model::AgentSelectionStage::AuthenticationRequired
                    | crate::model::AgentSelectionStage::Selected
                    | crate::model::AgentSelectionStage::Failed
            ),
            agent_verification_required: agent_selection.stage
                == crate::model::AgentSelectionStage::HistorySelected,
            claude_checked: selected == Some(AgentKind::Claude),
            codex_checked: selected == Some(AgentKind::Codex),
            selected_agent: selected,
            working_directory_text: menu_safe_path(&config.working_directory.to_string_lossy()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentMenuEntry {
    agent: AgentKind,
    label: String,
    checked: bool,
    enabled: bool,
    profile: Option<crate::model::ExternalAgentProfile>,
}

#[derive(Default)]
struct AgentMenuState {
    generation: Uuid,
    entries: Vec<AgentMenuEntry>,
}
impl AgentMenuState {
    fn choice(&self, id: &str, current: &[AgentMenuEntry]) -> Option<AgentKind> {
        let rest = id.strip_prefix("agent_choice:")?;
        let (generation, index) = rest.split_once(':')?;
        if Uuid::parse_str(generation).ok()? != self.generation || self.entries != current {
            return None;
        }
        self.entries
            .get(index.parse::<usize>().ok()?)
            .filter(|entry| entry.enabled)
            .map(|entry| entry.agent)
    }
}

fn agent_menu_entries(config: &AppConfig, view: &TrayMenuPresentation) -> Vec<AgentMenuEntry> {
    [AgentKind::Claude, AgentKind::Codex]
        .into_iter()
        .map(|agent| (agent, None))
        .chain(
            config
                .external_agents
                .iter()
                .map(|profile| (AgentKind::External(profile.id), Some(profile.clone()))),
        )
        .map(|(agent, profile)| {
            let checked = match agent {
                AgentKind::Claude => view.claude_checked,
                AgentKind::Codex => view.codex_checked,
                AgentKind::External(_) => view.selected_agent == Some(agent),
            };
            let name = profile
                .as_ref()
                .map(|profile| profile.name.as_str())
                .unwrap_or_else(|| crate::session_view::agent_label(agent));
            let suffix = if checked && view.agent_verification_required {
                " — Verify to Start"
            } else {
                ""
            };
            AgentMenuEntry {
                agent,
                label: format!("{}{suffix}", menu_safe_path(name)),
                checked,
                enabled: view.agent_selection_enabled,
                profile,
            }
        })
        .collect()
}

fn select_agent_menu_choice<R: tauri::Runtime>(app: &AppHandle<R>, id: &str) {
    let chosen = (|| {
        let snapshot = app.state::<crate::app_state::AppState>().snapshot().ok()?;
        let view = TrayMenuPresentation::derive(
            &snapshot.agent_selection,
            &snapshot.config,
            &snapshot.lens,
        );
        let current = agent_menu_entries(&snapshot.config, &view);
        app.state::<TrayMenuItems<R>>()
            .agent_presentation
            .lock()
            .ok()?
            .choice(id, &current)
            .map(|agent| (agent, snapshot.revision))
    })();
    // Native check items toggle before dispatch. Restore authoritative radio state even for stale/no-op clicks.
    let _ = sync_agent_menu(app);
    if let Some((agent, revision)) = chosen {
        select_agent_from_menu(app, agent, revision);
    }
}

fn sync_agent_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            let snapshot = handle.state::<crate::app_state::AppState>().snapshot()?;
            let view = TrayMenuPresentation::derive(
                &snapshot.agent_selection,
                &snapshot.config,
                &snapshot.lens,
            );
            let entries = agent_menu_entries(&snapshot.config, &view);
            let items = handle.state::<TrayMenuItems<R>>();
            let mut previous = items
                .agent_presentation
                .lock()
                .map_err(|_| "Agent menu is unavailable")?;
            let menu = &items.agents;
            if previous.entries == entries {
                for (index, item) in menu
                    .items()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .enumerate()
                {
                    if let (Some(check), Some(entry)) =
                        (item.as_check_menuitem(), entries.get(index))
                    {
                        check
                            .set_checked(entry.checked)
                            .map_err(|error| error.to_string())?;
                    }
                }
                return Ok(());
            }
            while !menu.items().map_err(|error| error.to_string())?.is_empty() {
                menu.remove_at(0).map_err(|error| error.to_string())?;
            }
            let generation = Uuid::new_v4();
            for (index, entry) in entries.iter().enumerate() {
                menu.append(
                    &CheckMenuItem::with_id(
                        &handle,
                        format!("agent_choice:{generation}:{index}"),
                        &entry.label,
                        entry.enabled,
                        entry.checked,
                        None::<&str>,
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            }
            menu.append(
                &PredefinedMenuItem::separator(&handle).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            menu.append(
                &MenuItem::with_id(
                    &handle,
                    "manage_agent_presets",
                    "Manage Presets…",
                    true,
                    None::<&str>,
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            *previous = AgentMenuState {
                generation,
                entries,
            };
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Unable to update Agent menu: {error}");
        }
    })
    .map_err(|error| error.to_string())
}

pub fn install_menu_bar<R: tauri::Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let select = MenuItem::with_id(
        app,
        "select_target",
        "Select Targets...",
        false,
        None::<&str>,
    )?;
    let agents = Submenu::with_id(app, "agents", "Agents", true)?;
    let directory_label = MenuItem::with_id(
        app,
        "directory_label",
        "Working Directory",
        false,
        None::<&str>,
    )?;
    let working_directory = MenuItem::with_id(
        app,
        "working_directory",
        "Working Directory…",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
    let about = MenuItem::with_id(app, "about", "About", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let prompt_presets = Submenu::with_id(app, "prompt_presets", "Prompt Presets", true)?;
    let history = Submenu::with_id(app, "session_history", "Recent Sessions", true)?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let separator_three = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &select,
            &history,
            &separator_one,
            &directory_label,
            &working_directory,
            &separator_two,
            &agents,
            &prompt_presets,
            &settings,
            &about,
            &separator_three,
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
            "refresh_session_history" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = crate::session_view::refresh(app).await {
                        eprintln!("Unable to refresh history: {error}");
                    }
                });
            }
            id if id.starts_with("session_history:") => {
                let mut parts = id.split(':').skip(1);
                if let (Some(generation), Some(index)) = (
                    parts.next().and_then(|value| Uuid::parse_str(value).ok()),
                    parts.next().and_then(|value| value.parse::<usize>().ok()),
                ) {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) =
                            crate::session_view::open(app.clone(), generation, index).await
                        {
                            eprintln!("Unable to open history: {error}");
                        }
                    });
                }
            }
            "select_target" => select_lens_target_from_tray(app),
            id if id.starts_with("agent_choice:") => select_agent_menu_choice(app, id),
            "manage_agent_presets" => {
                let _ = show_settings_destination(app, Some(SettingsDestination::Connection));
            }
            "working_directory" => choose_working_directory(app),
            "settings" => {
                if let Err(error) = show_settings(app) {
                    eprintln!("Unable to show Settings: {error}");
                }
            }
            "manage_prompt_presets" => {
                if let Err(error) = show_prompt_settings(app) {
                    eprintln!("Unable to show prompt presets: {error}");
                }
            }
            id if id.starts_with("prompt_preset:") => {
                let id = id.trim_start_matches("prompt_preset:").to_string();
                if let Err(error) = commands::update_prompt_presets(
                    usecase::prompt_presets::PromptPresetMutation::Select { id },
                    app.clone(),
                ) {
                    let _ = sync_prompt_preset_menu(app);
                    app.dialog()
                        .message(format!("Unable to change prompt preset: {error}"))
                        .title("Lens")
                        .show(|_| {});
                }
            }
            "about" => {
                if let Err(error) = show_about(app) {
                    eprintln!("Unable to show About: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(TrayMenuItems {
        root: menu,
        history,
        history_presentation: Mutex::new(String::new()),
        select_target: select,
        agents,
        agent_presentation: Mutex::new(AgentMenuState::default()),
        working_directory,
        prompt_presets,
        prompt_presentation: Mutex::new(Vec::new()),
    });
    sync_tray_menu(app.handle()).map_err(|error| tauri::Error::Io(std::io::Error::other(error)))?;
    let history_app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::session_view::refresh(history_app).await {
            eprintln!("Unable to refresh history: {error}");
        }
    });
    Ok(())
}

pub fn sync_tray_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let snapshot = state.snapshot()?;
    let presentation =
        TrayMenuPresentation::derive(&snapshot.agent_selection, &snapshot.config, &snapshot.lens);
    app.state::<TrayPresentation<R>>()
        .0
        .apply(app, presentation)
}

impl<R: tauri::Runtime> TrayOutput<R> for NativeTrayOutput {
    fn apply(&self, app: &AppHandle<R>, presentation: TrayMenuPresentation) -> Result<(), String> {
        sync_history_menu(app)?;
        sync_prompt_preset_menu(app)?;
        sync_agent_menu(app)?;
        let items = app.state::<TrayMenuItems<R>>();
        items
            .select_target
            .set_enabled(presentation.select_target_enabled)
            .map_err(|error| error.to_string())?;
        items
            .working_directory
            .set_text(presentation.working_directory_text)
            .map_err(|error| error.to_string())?;
        let tray = app
            .tray_by_id("lens")
            .ok_or_else(|| "Lens tray icon is unavailable".to_string())?;
        tray.set_icon_with_as_template(
            Some(
                tray_icon(presentation.select_target_enabled || presentation.live_lens_active)
                    .map_err(|error| error.to_string())?,
            ),
            true,
        )
        .map_err(|error| error.to_string())?;
        let tooltip = if presentation.target_selection_active {
            "Lens — Lens Target selection is already active"
        } else if presentation.live_lens_active {
            "Lens — left-click to show the active Lens"
        } else if presentation.select_target_enabled {
            "Lens — left-click to select Lens Targets"
        } else {
            "Lens — select and authenticate an AI Agent to enable target selection"
        };
        tray.set_tooltip(Some(tooltip))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn sync_history_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // Mock shells need no native menu. Real menu mutation stays on its owning thread.
    if app.try_state::<TrayMenuItems<R>>().is_none() {
        return Ok(());
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            let app = &handle;
            let state = app.state::<crate::app_state::AppState>();
            let mut catalog = state.session_view.catalog()?;
            let directory_matches = catalog.cwd == state.config()?.working_directory;
            if !directory_matches {
                catalog.entries.clear();
                catalog.notices.clear();
            }
            let enabled = directory_matches && crate::session_view::history_enabled(app)?;
            let key = format!(
                "{}:{}:{enabled}:{directory_matches}",
                catalog.generation, catalog.loading
            );
            let items = app.state::<TrayMenuItems<R>>();
            let mut shown = items
                .history_presentation
                .lock()
                .map_err(|_| "History menu lock is poisoned")?;
            if *shown == key {
                return Ok(());
            }
            let menu = &items.history;
            while !menu.items().map_err(|error| error.to_string())?.is_empty() {
                menu.remove_at(0).map_err(|error| error.to_string())?;
            }
            let mut tooltips = Vec::with_capacity(catalog.entries.len());
            for (index, entry) in catalog.entries.iter().enumerate() {
                let date = entry
                    .updated_at
                    .as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|date| {
                        app.state::<crate::platform::Presentation<R>>()
                            .0
                            .format_short_datetime(date.timestamp() as f64)
                    })
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default();
                let title: String = entry.title.chars().take(72).collect();
                let label = menu_safe_path(&title);
                let config = app.state::<crate::app_state::AppState>().config()?;
                let agent = match entry.agent {
                    AgentKind::External(id) => config
                        .external_agents
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.as_str())
                        .unwrap_or("External ACP"),
                    managed => crate::session_view::agent_label(managed),
                };
                tooltips.push(if date.is_empty() {
                    agent.to_string()
                } else {
                    format!("{date} · {agent}")
                });
                menu.append(
                    &MenuItem::with_id(
                        app,
                        format!("session_history:{}:{index}", catalog.generation),
                        label,
                        enabled && entry.can_load,
                        None::<&str>,
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            }
            if catalog.entries.is_empty() {
                menu.append(
                    &MenuItem::with_id(
                        app,
                        "history_empty",
                        if catalog.loading {
                            "Loading sessions…"
                        } else {
                            "No recent sessions"
                        },
                        false,
                        None::<&str>,
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            }
            for (index, notice) in catalog.notices.iter().enumerate() {
                let notice: String = notice.chars().take(120).collect();
                menu.append(
                    &MenuItem::with_id(
                        app,
                        format!("history_notice:{index}"),
                        menu_safe_path(&notice),
                        false,
                        None::<&str>,
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            }
            menu.append(&PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            menu.append(
                &MenuItem::with_id(
                    app,
                    "refresh_session_history",
                    if catalog.loading {
                        "Refreshing…"
                    } else {
                        "Refresh Sessions"
                    },
                    !catalog.loading,
                    None::<&str>,
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            app.state::<crate::platform::Presentation<R>>()
                .0
                .history_tooltips(app, &items.root, menu, tooltips)
                .map_err(|error| error.to_string())?;
            *shown = key;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Unable to synchronize history menu: {error}");
        }
    })
    .map_err(|error| error.to_string())
}

pub(crate) fn show_history_window<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LENS_WINDOW_LABEL) {
        return present_window(&window);
    }
    let geometry = primary_screen_overlay_geometry(app)?;
    let window = overlay_window_builder(app)
        .inner_size(geometry.width, geometry.height)
        .position(geometry.x, geometry.y)
        .build()?;
    present_window(&window)
}

/// Live and restored sessions share one native surface; only placement differs.
fn overlay_window_builder<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> WebviewWindowBuilder<'_, R, AppHandle<R>> {
    let link_app = app.clone();
    WebviewWindowBuilder::new(app, LENS_WINDOW_LABEL, webview_url(WebviewView::Overlay))
        .on_new_window(move |url, _| crate::html_preview::open_link(&link_app, &url))
        .title("Lens")
        .min_inner_size(360.0, 320.0)
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
}

fn sync_prompt_preset_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            // Read the latest state on the menu's owning thread: queued updates cannot rebuild
            // an older catalog over a newer one, and native calls never wait holding a worker lock.
            let config = handle.state::<crate::app_state::AppState>().config()?;
            let presentation = prompt_preset_menu(&config);
            let items = handle.state::<TrayMenuItems<R>>();
            let mut previous = items
                .prompt_presentation
                .lock()
                .map_err(|_| "prompt menu lock is poisoned")?;
            let menu = &items.prompt_presets;
            if *previous == presentation {
                // Native check items toggle before dispatching their event, including a
                // no-op selection. Reapply saved radio state even when labels did not change.
                for item in menu.items().map_err(|error| error.to_string())? {
                    if let Some(check) = item.as_check_menuitem() {
                        let checked = presentation.iter().any(|preset| {
                            item.id().as_ref() == format!("prompt_preset:{}", preset.id)
                                && preset.checked
                        });
                        check
                            .set_checked(checked)
                            .map_err(|error| error.to_string())?;
                    }
                }
                return Ok(());
            }
            while !menu.items().map_err(|error| error.to_string())?.is_empty() {
                menu.remove_at(0).map_err(|error| error.to_string())?;
            }
            for preset in &presentation {
                let item = CheckMenuItem::with_id(
                    &handle,
                    format!("prompt_preset:{}", preset.id),
                    &preset.name,
                    true,
                    preset.checked,
                    None::<&str>,
                )
                .map_err(|error| error.to_string())?;
                menu.append(&item).map_err(|error| error.to_string())?;
            }
            menu.append(
                &PredefinedMenuItem::separator(&handle).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            menu.append(
                &MenuItem::with_id(
                    &handle,
                    "manage_prompt_presets",
                    "Manage Presets…",
                    true,
                    None::<&str>,
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            *previous = presentation;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Unable to update prompt preset menu: {error}");
        }
    })
    .map_err(|error| error.to_string())
}

fn select_lens_target_from_tray<R: tauri::Runtime>(app: &AppHandle<R>) {
    let snapshot = app.state::<crate::app_state::AppState>().snapshot();
    let Ok(snapshot) = snapshot else {
        return;
    };
    if snapshot.lens.live.is_some() {
        if let Some(target_set) = snapshot.lens.target_set.as_ref() {
            if let Err(error) = show_lens_window(app, target_set) {
                eprintln!("Unable to show the active Lens: {error}");
            }
        }
        return;
    }
    let enabled = snapshot.agent_selection.can_select_lens_target()
        && snapshot.lens.stage != LensStage::Selecting;
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

fn select_agent_from_menu<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: AgentKind,
    expected_revision: u32,
) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match crate::agent::select_agent_guarded(handle.clone(), agent, Some(expected_revision))
            .await
        {
            Ok(selection) if selection.selected_agent().is_some() => {}
            Ok(_) => {
                if let Err(error) =
                    show_settings_destination(&handle, Some(SettingsDestination::Connection))
                {
                    eprintln!("Unable to show Agent authentication in Settings: {error}");
                }
            }
            Err(error) => {
                eprintln!("Unable to select Agent: {error}");
                if let Err(settings_error) =
                    show_settings_destination(&handle, Some(SettingsDestination::Connection))
                {
                    eprintln!("Unable to show Agent error in Settings: {settings_error}");
                }
            }
        }
    });
}

fn choose_working_directory<R: tauri::Runtime>(app: &AppHandle<R>) {
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

fn settings_background<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> tauri::Result<tauri::utils::config::Color> {
    app.state::<crate::platform::Presentation<R>>()
        .0
        .settings_background(app)
        .map_err(|error| tauri::Error::Io(std::io::Error::other(error.to_string())))
}

/// Keep the native and WebView surfaces in the same appearance as HTML system colors.
/// Store a label rather than retaining a window in its own event listener.
fn track_settings_background<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    window.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::ThemeChanged(_)) {
            return;
        }
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) =
                settings_background(&app).and_then(|color| window.set_background_color(Some(color)))
            {
                eprintln!("Unable to update {label} background: {error}");
            }
        }
    });
}

pub fn show_about<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("about") {
        return present_window(&window).map_err(|error| error.to_string());
    }
    let background = settings_background(app).map_err(|error| error.to_string())?;
    let window = WebviewWindowBuilder::new(app, "about", webview_url(WebviewView::About))
        .background_color(background)
        .title("About Lens")
        .minimizable(false)
        .inner_size(640.0, 560.0)
        .min_inner_size(400.0, 320.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|error| error.to_string())?;
    track_settings_background(&window);
    present_window(&window).map_err(|error| error.to_string())
}

pub fn show_settings<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    show_settings_destination(app, None)
}

fn show_prompt_settings<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    show_settings_destination(app, Some(SettingsDestination::PromptPresets))
}

#[derive(Clone, Copy)]
enum SettingsDestination {
    Connection,
    PromptPresets,
}

impl SettingsDestination {
    fn id(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::PromptPresets => "prompt-presets",
        }
    }
}

fn show_settings_destination<R: tauri::Runtime>(
    app: &AppHandle<R>,
    destination: Option<SettingsDestination>,
) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        present_window(&window)?;
        if let Some(destination) = destination {
            window.emit("settings-destination", destination.id())?;
        }
        return Ok(());
    }

    let size = app
        .primary_monitor()?
        .map(|monitor| {
            SETTINGS_WINDOW_SIZE_POLICY
                .initial_size(monitor.work_area().size.to_logical(monitor.scale_factor()))
        })
        .unwrap_or(SETTINGS_WINDOW_SIZE_POLICY.preferred);

    let background = settings_background(app)?;
    let url = if let Some(destination) = destination {
        WebviewUrl::App(
            format!(
                "settings.html?platform={DESKTOP_PLATFORM}&destination={}",
                destination.id()
            )
            .into(),
        )
    } else {
        webview_url(WebviewView::Settings)
    };
    let window = WebviewWindowBuilder::new(app, SETTINGS_LABEL, url)
        .background_color(background)
        .title("Lens Settings")
        .minimizable(false)
        .inner_size(size.width, size.height)
        .inner_size_constraints(SETTINGS_WINDOW_SIZE_POLICY.constraints())
        .resizable(true)
        .maximizable(SETTINGS_WINDOW_SIZE_POLICY.maximizable)
        .center()
        .build()?;
    track_settings_background(&window);
    present_window(&window)
}

pub fn show_lens_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
    target_set: &LensTargetSet,
) -> tauri::Result<()> {
    let window = app.get_webview_window(LENS_WINDOW_LABEL);
    let presentation = app.state::<LensWindowPresentationState>();
    let mut placement = presentation.placement.lock().map_err(|_| {
        tauri::Error::Io(std::io::Error::other(
            "Lens window placement state lock is poisoned",
        ))
    })?;
    let decision = placement.decision(target_set.selection_id, window.is_some());

    match (window, decision) {
        (Some(window), LensWindowPlacementDecision::Apply) => {
            let geometry = lens_window_geometry(app, target_set)?;
            window.set_size(LogicalSize::new(geometry.width, geometry.height))?;
            window.set_position(LogicalPosition::new(geometry.x, geometry.y))?;
            placement.record_applied(target_set.selection_id);
            drop(placement);
            present_window(&window)
        }
        (Some(window), LensWindowPlacementDecision::Preserve) => {
            drop(placement);
            present_window(&window)
        }
        (None, LensWindowPlacementDecision::Apply) => {
            let geometry = lens_window_geometry(app, target_set)?;
            let window = overlay_window_builder(app)
                .inner_size(geometry.width, geometry.height)
                .position(geometry.x, geometry.y)
                .build()?;
            placement.record_applied(target_set.selection_id);
            drop(placement);
            present_window(&window)
        }
        (None, LensWindowPlacementDecision::Preserve) => Err(tauri::Error::Io(
            std::io::Error::other("missing Lens window cannot preserve placement"),
        )),
    }
}

pub async fn show_target_selection_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
        crate::platform::transition_window_frame(
            &window,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
        )
        .await
        .map_err(|error| tauri::Error::Io(std::io::Error::other(error.to_string())))?;
        return present_window(&window);
    }

    let window = WebviewWindowBuilder::new(
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
    .visible(false)
    .accept_first_mouse(true)
    .effects(WindowEffectsConfig {
        effects: vec![LENS_WINDOW_EFFECT],
        state: Some(WindowEffectState::Active),
        radius: Some(LENS_WINDOW_CORNER_RADIUS),
        color: None,
    })
    .build()?;
    crate::platform::present_window_from_screen_right(&window)
        .map_err(|error| tauri::Error::Io(std::io::Error::other(error.to_string())))?;
    // The native entrance owns showing and positioning; focus only after it completes.
    window.set_focus()
}

pub async fn dismiss_target_selection_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(TARGET_SELECTION_WINDOW_LABEL) {
        crate::platform::dismiss_window_to_screen_right(&window)
            .await
            .map_err(|error| tauri::Error::Io(std::io::Error::other(error.to_string())))?;
        window.destroy()?;
    }
    Ok(())
}

pub fn destroy_target_selection_window<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
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
    fn prompt_menu_has_one_saved_selection_and_literal_user_names() {
        let mut config = crate::store::default_config();
        config.prompt_presets.presets[0].name = "A & B".into();
        let items = prompt_preset_menu(&config);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].name, "A && B");
        assert_eq!(items.iter().filter(|item| item.checked).count(), 1);
        assert_eq!(
            items.iter().find(|item| item.checked).unwrap().id,
            config.prompt_presets.selected_id
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            config
                .prompt_presets
                .presets
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>()
        );
    }

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
    fn settings_width_constraints_keep_the_sidebar_and_bound_unused_space() {
        let constraints = SETTINGS_WINDOW_SIZE_POLICY.constraints();

        assert_eq!(constraints.min_width, Some(LogicalUnit::new(715.0).into()));
        assert_eq!(constraints.min_height, Some(LogicalUnit::new(360.0).into()));
        assert_eq!(constraints.max_width, Some(LogicalUnit::new(920.0).into()));
        assert_eq!(constraints.max_height, None);
        assert!(SETTINGS_WINDOW_SIZE_POLICY.resizable_width_range_is_valid());
    }

    #[test]
    fn webview_urls_publish_explicit_finite_presentation_state() {
        let WebviewUrl::App(about) = webview_url(WebviewView::About) else {
            panic!("About must use an application WebView URL");
        };
        assert_eq!(
            about,
            std::path::PathBuf::from(format!("about.html?platform={DESKTOP_PLATFORM}"))
        );
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
            std::path::PathBuf::from(format!("settings.html?platform={DESKTOP_PLATFORM}"))
        );
        assert_eq!(
            overlay,
            std::path::PathBuf::from(format!("overlay.html?platform={DESKTOP_PLATFORM}"))
        );
        assert_eq!(
            target_selection,
            std::path::PathBuf::from(format!("target-selection.html?platform={DESKTOP_PLATFORM}"))
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
    fn single_target_lens_window_is_eighty_percent_and_centered_in_target_coordinates() {
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
    fn multiple_target_lens_window_is_portrait_and_centered_on_the_primary_screen() {
        let geometry = LensWindowGeometry::from_primary_screen(
            LogicalPosition::new(-1920.0, 0.0),
            LogicalSize::new(1920.0, 1200.0),
        )
        .expect("valid primary screen geometry");

        assert_eq!(
            geometry,
            LensWindowGeometry {
                x: -1260.0,
                y: 120.0,
                width: 600.0,
                height: 960.0,
            }
        );
        assert_eq!(geometry.height / 1200.0, 0.8);
        assert_eq!(geometry.width / geometry.height, 10.0 / 16.0);
    }

    #[test]
    fn lens_window_placement_is_scoped_to_selection_identity() {
        let first_selection = Uuid::new_v4();
        let second_selection = Uuid::new_v4();
        let mut placement = LensWindowPlacementState::default();

        assert_eq!(
            placement.decision(first_selection, false),
            LensWindowPlacementDecision::Apply
        );
        placement.record_applied(first_selection);
        assert_eq!(
            placement.decision(first_selection, true),
            LensWindowPlacementDecision::Preserve,
            "content revisions within one selection must preserve user geometry"
        );
        assert_eq!(
            placement.decision(second_selection, true),
            LensWindowPlacementDecision::Apply,
            "a newly confirmed target set must reapply its placement"
        );
        assert_eq!(
            placement.decision(first_selection, false),
            LensWindowPlacementDecision::Apply,
            "a destroyed window must apply geometry when recreated"
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
        assert_eq!(
            LensWindowGeometry::from_primary_screen(
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1440.0, 0.0),
            ),
            None
        );
        assert_eq!(
            LensWindowGeometry::from_primary_screen(
                LogicalPosition::new(f64::NAN, 0.0),
                LogicalSize::new(1440.0, 900.0),
            ),
            None
        );
    }

    #[test]
    fn tray_agent_status_and_managed_checks_share_one_selection() {
        let config = AppConfig::new(PathBuf::from("/tmp"));
        let external = &config.external_agents[0];
        for stage in [
            AgentSelectionStage::Selected,
            AgentSelectionStage::HistorySelected,
        ] {
            for agent in [
                AgentKind::Claude,
                AgentKind::Codex,
                AgentKind::External(external.id),
            ] {
                let state = AgentSelectionState {
                    stage,
                    candidate: Some(agent),
                    ..Default::default()
                };
                let view = TrayMenuPresentation::derive(&state, &config, &LensState::default());
                assert_eq!(view.claude_checked, agent == AgentKind::Claude);
                assert_eq!(view.codex_checked, agent == AgentKind::Codex);
                assert!(!(view.claude_checked && view.codex_checked));
                assert_eq!(view.selected_agent, Some(agent));
                let entries = agent_menu_entries(&config, &view);
                assert_eq!(entries.iter().filter(|entry| entry.checked).count(), 1);
                assert_eq!(
                    entries.iter().find(|entry| entry.checked).unwrap().agent,
                    agent
                );
            }
        }
        let view = TrayMenuPresentation::derive(
            &AgentSelectionState::default(),
            &config,
            &LensState::default(),
        );
        assert_eq!(view.selected_agent, None);
        assert!(!view.claude_checked && !view.codex_checked);
    }

    #[test]
    fn agent_menu_preserves_saved_order_and_rejects_stale_or_disabled_choices() {
        let mut config = AppConfig::new(PathBuf::from("/tmp"));
        config.external_agents.reverse();
        config.external_agents[0].name = "Z first".into();
        config.external_agents[1].name = "A second".into();
        let selected = AgentKind::External(config.external_agents[1].id);
        let selection = AgentSelectionState {
            stage: AgentSelectionStage::Selected,
            candidate: Some(selected),
            ..Default::default()
        };
        let view = TrayMenuPresentation::derive(&selection, &config, &LensState::default());
        let entries = agent_menu_entries(&config, &view);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["Claude", "Codex", "Z first", "A second"]
        );
        assert_eq!(entries.iter().filter(|entry| entry.checked).count(), 1);
        let generation = Uuid::new_v4();
        let menu = AgentMenuState {
            generation,
            entries: entries.clone(),
        };
        let id = format!("agent_choice:{generation}:3");
        assert_eq!(menu.choice(&id, &entries), Some(selected));
        assert_eq!(
            menu.choice(&format!("agent_choice:{}:3", Uuid::new_v4()), &entries),
            None
        );
        assert_eq!(
            menu.choice(&format!("agent_choice:{generation}:999"), &entries),
            None
        );
        for modification in 0..3 {
            let mut changed = config.clone();
            match modification {
                0 => {
                    changed.external_agents.remove(1);
                }
                1 => changed.external_agents[1].command = "replacement".into(),
                _ => changed.external_agents[1].args.push("changed".into()),
            }
            assert_eq!(menu.choice(&id, &agent_menu_entries(&changed, &view)), None);
        }
        let mut disabled = entries;
        disabled[3].enabled = false;
        let menu = AgentMenuState {
            generation,
            entries: disabled.clone(),
        };
        assert_eq!(menu.choice(&id, &disabled), None);
    }

    #[test]
    fn tray_presentation_separates_chosen_agent_from_execution_readiness() {
        let config = AppConfig {
            agent: AgentKind::Codex,
            external_agents: Vec::new(),
            working_directory: PathBuf::from("/Users/example/Work"),
            agent_prompt_template: crate::store::default_config().agent_prompt_template,
            prompt_presets: crate::store::default_config().prompt_presets,
            agent_preferences: Default::default(),
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
                live_lens_active: false,
                agent_selection_enabled: true,
                agent_verification_required: false,
                claude_checked: false,
                codex_checked: true,
                selected_agent: Some(AgentKind::Codex),
                working_directory_text: "/Users/example/Work".into(),
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

        let history_selected = AgentSelectionState {
            stage: AgentSelectionStage::HistorySelected,
            candidate: Some(AgentKind::Claude),
            ..AgentSelectionState::default()
        };
        let history =
            TrayMenuPresentation::derive(&history_selected, &config, &LensState::default());
        assert!(history.claude_checked);
        assert!(!history.codex_checked);
        assert!(history.agent_selection_enabled);
        assert!(history.agent_verification_required);
        assert!(!history.select_target_enabled);

        let selecting = LensState {
            stage: LensStage::Selecting,
            ..LensState::default()
        };
        let presentation = TrayMenuPresentation::derive(&selected, &config, &selecting);
        assert!(!presentation.select_target_enabled);
        assert!(presentation.target_selection_active);

        let active_lens = LensState {
            live: Some(crate::model::LensLiveState {
                lifecycle: crate::model::LensMonitoringLifecycle::Watching,
                health: crate::model::LensSourceHealth::Healthy,
                freshness: crate::model::LensFreshness::Current,
                agent_refresh_interval_seconds: crate::model::LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
                last_outcome: None,
                error: None,
            }),
            ..LensState::default()
        };
        let presentation = TrayMenuPresentation::derive(&selected, &config, &active_lens);
        assert!(!presentation.select_target_enabled);
        assert!(presentation.live_lens_active);
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

        let alpha_at = |x: u32, y: u32| {
            let alpha_index = ((y * icon.width() + x) * 4 + 3) as usize;
            icon.rgba()[alpha_index]
        };
        assert_eq!(
            alpha_at(18, 4),
            u8::MAX,
            "the circular frame must remain opaque"
        );
        assert!(
            [alpha_at(22, 14), alpha_at(11, 19)]
                .into_iter()
                .all(|alpha| alpha == u8::MAX),
            "both canonical lens surfaces must remain opaque"
        );
        assert_eq!(
            alpha_at(18, 7),
            0,
            "the frame and lens must remain separated by transparent space"
        );
        assert!(
            [
                alpha_at(15, 14),
                alpha_at(20, 21),
                alpha_at(32, 24),
                alpha_at(32, 28)
            ]
            .into_iter()
            .all(|alpha| alpha == 0),
            "the L reflection and retired chain area must remain transparent"
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
