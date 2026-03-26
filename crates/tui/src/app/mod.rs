mod helpers;
mod input;
mod lifecycle;
mod render;
mod terminal;

#[cfg(test)]
mod tests;

use crate::artifact_store::{RunDetailRecord, RunSummaryRecord, discover_runs};
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_engine::RunEngine;
use nao_engine::RunExecutionResult;
use nao_pal::pal::PalHandle;
use nao_recipe::Task;
use std::collections::BTreeSet;
use std::sync::mpsc::Receiver;
use tui_scrollview::ScrollViewState;

pub(crate) use helpers::{
    adjust_scroll, centered_rect, focused_block, pretty_duration, render_task_state_emoji,
    spinner_frames, top_level_layout,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Launcher,
    RunDetail,
    RunHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    LauncherTasks,
    LauncherDetails,
    LauncherFailureOutput,
    HistoryRuns,
    HistoryDetails,
    DetailTasks,
    DetailOutput,
    DetailEvents,
    DetailSummary,
}

#[derive(Debug)]
struct ActiveRunHandle {
    run_directory: FilePath,
    receiver: Receiver<NaoResult<RunExecutionResult>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct OpenRunRefreshState {
    force_detail_reload: bool,
    force_selected_task_log_reload: bool,
    force_launcher_failed_log_reload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunDetailRefreshOutcome {
    selected_task_changed: bool,
    failed_task_changed: bool,
}

/// Owns TUI state, rendering, and keyboard routing.
pub struct App {
    pal: PalHandle,
    engine: RunEngine,
    recipe_path: FilePath,
    screen: Screen,
    focus: Focus,
    help_visible: bool,
    tasks: Vec<Task>,
    selected_task_index: usize,
    selected_goals: BTreeSet<SharedString>,
    runs: Vec<RunSummaryRecord>,
    selected_run_index: usize,
    open_run_directory: Option<FilePath>,
    run_detail: Option<RunDetailRecord>,
    launcher_failed_task_name: Option<SharedString>,
    launcher_failed_task_log_lines: Vec<SharedString>,
    launcher_log_scroll_state: ScrollViewState,
    selected_run_task_index: usize,
    task_log_lines: Vec<SharedString>,
    log_scroll_state: ScrollViewState,
    events_scroll: u16,
    summary_scroll: u16,
    auto_follow_log: bool,
    status_message: Option<SharedString>,
    active_run: Option<ActiveRunHandle>,
    launched_run_in_session: bool,
    spinner_frame: usize,
    refresh_tick: u64,
    open_run_refresh_state: OpenRunRefreshState,
}

impl App {
    /// Creates a new app and loads launcher and run history state.
    pub fn new(pal: PalHandle, recipe_path: FilePath) -> NaoResult<Self> {
        let engine = RunEngine::new(pal.clone());
        let tasks = engine.list_tasks(&recipe_path)?;
        let runs = discover_runs(&*pal, &recipe_path)?;
        let app = Self {
            pal,
            engine,
            recipe_path,
            screen: Screen::Launcher,
            focus: Focus::LauncherTasks,
            help_visible: false,
            tasks,
            selected_task_index: 0,
            selected_goals: BTreeSet::new(),
            runs,
            selected_run_index: 0,
            open_run_directory: None,
            run_detail: None,
            launcher_failed_task_name: None,
            launcher_failed_task_log_lines: Vec::new(),
            launcher_log_scroll_state: ScrollViewState::new(),
            selected_run_task_index: 0,
            task_log_lines: Vec::new(),
            log_scroll_state: ScrollViewState::new(),
            events_scroll: 0,
            summary_scroll: 0,
            auto_follow_log: true,
            status_message: None,
            active_run: None,
            launched_run_in_session: false,
            spinner_frame: 0,
            refresh_tick: 0,
            open_run_refresh_state: OpenRunRefreshState::default(),
        };
        Ok(app)
    }
}
