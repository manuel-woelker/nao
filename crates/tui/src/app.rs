use crate::artifact_store::{
    RunDetailRecord, RunSummaryRecord, discover_runs, load_run_detail, load_task_log_lines,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_engine::PlannedRun;
use nao_engine::RunEngine;
use nao_engine::RunExecutionResult;
use nao_engine::run_artifact_writer::RunArtifactWriter;
use nao_pal::pal::PalHandle;
use nao_recipe::Task;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, StatefulWidget, Tabs, Wrap,
};
use std::collections::BTreeSet;
use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

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
        };
        Ok(app)
    }

    /// Starts the full-screen terminal event loop.
    pub fn run(&mut self) -> NaoResult<()> {
        let mut terminal = TerminalSession::enter()?;
        loop {
            self.refresh()?;
            terminal.terminal.draw(|frame| self.render(frame))?;
            if event::poll(Duration::from_millis(150))?
                && let Event::Key(key_event) = event::read()?
                && self.handle_key_event(key_event)?
            {
                break;
            }
        }
        Ok(())
    }

    fn refresh(&mut self) -> NaoResult<()> {
        self.spinner_frame = (self.spinner_frame + 1) % spinner_frames().len();
        self.refresh_active_run()?;
        if self.open_run_directory.is_some() {
            self.refresh_open_run_detail()?;
        }
        Ok(())
    }

    fn refresh_active_run(&mut self) -> NaoResult<()> {
        let Some(active_run) = &self.active_run else {
            return Ok(());
        };
        match active_run.receiver.try_recv() {
            Ok(Ok(_result)) => {
                self.status_message = Some(SharedString::from("run completed"));
                let completed_run_directory = active_run.run_directory.clone();
                self.active_run = None;
                self.reload_history()?;
                self.open_run(&completed_run_directory)?;
            }
            Ok(Err(error)) => {
                self.status_message = Some(SharedString::from(error.to_test_string()));
                self.active_run = None;
                self.reload_history()?;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.active_run = None;
            }
        }
        Ok(())
    }

    fn reload_history(&mut self) -> NaoResult<()> {
        self.runs = discover_runs(&*self.pal, &self.recipe_path)?;
        if self.selected_run_index >= self.runs.len() && !self.runs.is_empty() {
            self.selected_run_index = self.runs.len() - 1;
        }
        Ok(())
    }

    fn refresh_open_run_detail(&mut self) -> NaoResult<()> {
        let Some(run_directory) = &self.open_run_directory else {
            return Ok(());
        };
        self.run_detail = Some(load_run_detail(&*self.pal, run_directory)?);
        self.reload_launcher_failed_task_log()?;
        if let Some(detail) = &self.run_detail {
            if detail.tasks.is_empty() {
                self.selected_run_task_index = 0;
                self.task_log_lines.clear();
            } else {
                self.selected_run_task_index = self
                    .selected_run_task_index
                    .min(detail.tasks.len().saturating_sub(1));
                self.reload_selected_task_log()?;
            }
        }
        Ok(())
    }

    fn reload_launcher_failed_task_log(&mut self) -> NaoResult<()> {
        let Some(detail) = &self.run_detail else {
            self.launcher_failed_task_name = None;
            self.launcher_failed_task_log_lines.clear();
            return Ok(());
        };
        let Some(task) = detail
            .tasks
            .iter()
            .find(|task| task.status.as_str() == "failed")
        else {
            self.launcher_failed_task_name = None;
            self.launcher_failed_task_log_lines.clear();
            return Ok(());
        };
        let failed_task_changed = self.launcher_failed_task_name.as_ref() != Some(&task.name);
        self.launcher_failed_task_name = Some(task.name.clone());
        self.launcher_failed_task_log_lines =
            load_task_log_lines(&*self.pal, &detail.run_directory, &task.log_file)?;
        if failed_task_changed {
            self.launcher_log_scroll_state.scroll_to_bottom();
        }
        Ok(())
    }

    fn reload_selected_task_log(&mut self) -> NaoResult<()> {
        let Some(detail) = &self.run_detail else {
            self.task_log_lines.clear();
            return Ok(());
        };
        let Some(task) = detail.tasks.get(self.selected_run_task_index) else {
            self.task_log_lines.clear();
            return Ok(());
        };
        self.task_log_lines =
            load_task_log_lines(&*self.pal, &detail.run_directory, &task.log_file)?;
        if self.auto_follow_log {
            self.log_scroll_state.scroll_to_bottom();
        }
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> NaoResult<bool> {
        if self.help_visible {
            match key_event.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.help_visible = false;
                }
                _ => {}
            }
            return Ok(false);
        }

        match key_event.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('?') => {
                self.help_visible = true;
                return Ok(false);
            }
            KeyCode::Char('1') => {
                self.screen = Screen::Launcher;
                self.focus = Focus::LauncherTasks;
                return Ok(false);
            }
            KeyCode::Char('2') => {
                self.screen = Screen::RunDetail;
                self.focus = Focus::DetailOutput;
                return Ok(false);
            }
            KeyCode::Char('3') => {
                self.screen = Screen::RunHistory;
                self.focus = Focus::HistoryRuns;
                return Ok(false);
            }
            KeyCode::Tab => {
                self.cycle_focus(false);
                return Ok(false);
            }
            KeyCode::BackTab => {
                self.cycle_focus(true);
                return Ok(false);
            }
            _ => {}
        }

        match self.screen {
            Screen::Launcher => self.handle_launcher_key(key_event)?,
            Screen::RunHistory => self.handle_history_key(key_event)?,
            Screen::RunDetail => self.handle_detail_key(key_event)?,
        }

        Ok(false)
    }

    fn handle_launcher_key(&mut self, key_event: KeyEvent) -> NaoResult<()> {
        match key_event.code {
            KeyCode::Char('o') if self.show_launcher_failure_output() => {
                self.focus = Focus::LauncherFailureOutput;
            }
            KeyCode::Char('g') if key_event.modifiers.is_empty() => {
                self.scroll_launcher_pane_to_top();
            }
            KeyCode::Char('G') | KeyCode::End => self.scroll_launcher_pane_to_bottom(),
            KeyCode::PageDown => self.scroll_launcher_pane(10),
            KeyCode::PageUp => self.scroll_launcher_pane(-10),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_launcher_pane(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_launcher_pane(-1),
            KeyCode::Char(' ') => self.toggle_selected_goal(),
            KeyCode::Enter => self.start_run()?,
            KeyCode::Char('r') => {
                self.reload_history()?;
                self.screen = Screen::RunHistory;
                self.focus = Focus::HistoryRuns;
            }
            _ => {}
        }
        Ok(())
    }

    fn scroll_launcher_pane(&mut self, delta: i32) {
        match self.focus {
            Focus::LauncherTasks => self.move_launcher_selection(delta as isize),
            Focus::LauncherFailureOutput => self.adjust_launcher_log_scroll(delta),
            _ => {}
        }
    }

    fn scroll_launcher_pane_to_top(&mut self) {
        if self.focus == Focus::LauncherFailureOutput {
            self.launcher_log_scroll_state.scroll_to_top();
        }
    }

    fn scroll_launcher_pane_to_bottom(&mut self) {
        if self.focus == Focus::LauncherFailureOutput {
            self.launcher_log_scroll_state.scroll_to_bottom();
        }
    }

    fn handle_history_key(&mut self, key_event: KeyEvent) -> NaoResult<()> {
        match key_event.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_history_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_history_selection(-1),
            KeyCode::Enter => {
                if let Some(run) = self.runs.get(self.selected_run_index) {
                    let run_directory = run.run_directory.clone();
                    self.open_run(&run_directory)?;
                    self.screen = Screen::RunDetail;
                    self.focus = Focus::DetailOutput;
                }
            }
            KeyCode::Char('R') => self.reload_history()?,
            KeyCode::Char('l') => {
                self.screen = Screen::Launcher;
                self.focus = Focus::LauncherTasks;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_detail_key(&mut self, key_event: KeyEvent) -> NaoResult<()> {
        match key_event.code {
            KeyCode::Char('t') => self.focus = Focus::DetailTasks,
            KeyCode::Char('o') => self.focus = Focus::DetailOutput,
            KeyCode::Char('e') => self.focus = Focus::DetailEvents,
            KeyCode::Char('s') => self.focus = Focus::DetailSummary,
            KeyCode::Char('r') => {
                self.screen = Screen::RunHistory;
                self.focus = Focus::HistoryRuns;
            }
            KeyCode::Char('L') => self.auto_follow_log = !self.auto_follow_log,
            KeyCode::Char('h') => self.cycle_focus(true),
            KeyCode::Char('l') => self.cycle_focus(false),
            KeyCode::Char('g') if key_event.modifiers.is_empty() => {
                self.scroll_current_pane_to_top()
            }
            KeyCode::Char('G') | KeyCode::End => self.scroll_current_pane_to_bottom(),
            KeyCode::PageDown => self.scroll_current_pane(10),
            KeyCode::PageUp => self.scroll_current_pane(-10),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_current_pane(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_current_pane(-1),
            KeyCode::Enter => self.reload_selected_task_log()?,
            _ => {}
        }
        Ok(())
    }

    fn move_launcher_selection(&mut self, delta: isize) {
        if self.tasks.is_empty() {
            self.selected_task_index = 0;
            return;
        }
        let next = self.selected_task_index as isize + delta;
        self.selected_task_index = next.clamp(0, self.tasks.len() as isize - 1) as usize;
    }

    fn toggle_selected_goal(&mut self) {
        let Some(task) = self.tasks.get(self.selected_task_index) else {
            return;
        };
        if !self.selected_goals.remove(&task.name.0) {
            self.selected_goals.insert(task.name.0.clone());
        }
    }

    fn move_history_selection(&mut self, delta: isize) {
        if self.runs.is_empty() {
            self.selected_run_index = 0;
            return;
        }
        let next = self.selected_run_index as isize + delta;
        self.selected_run_index = next.clamp(0, self.runs.len() as isize - 1) as usize;
    }

    fn scroll_current_pane(&mut self, delta: i32) {
        match self.focus {
            Focus::DetailTasks => self.move_selected_run_task(delta),
            Focus::DetailOutput => {
                if delta != 0 {
                    self.auto_follow_log = false;
                }
                self.adjust_log_scroll(delta);
            }
            Focus::DetailEvents => {
                self.events_scroll =
                    adjust_scroll(self.events_scroll, delta, self.max_events_scroll());
            }
            Focus::DetailSummary => {
                self.summary_scroll =
                    adjust_scroll(self.summary_scroll, delta, self.max_summary_scroll());
            }
            _ => {}
        }
    }

    fn scroll_current_pane_to_top(&mut self) {
        match self.focus {
            Focus::DetailOutput => {
                self.auto_follow_log = false;
                self.log_scroll_state.scroll_to_top();
            }
            Focus::DetailEvents => self.events_scroll = 0,
            Focus::DetailSummary => self.summary_scroll = 0,
            Focus::DetailTasks => self.move_selected_run_task_to(0),
            _ => {}
        }
    }

    fn scroll_current_pane_to_bottom(&mut self) {
        match self.focus {
            Focus::DetailOutput => {
                self.auto_follow_log = false;
                self.log_scroll_state.scroll_to_bottom();
            }
            Focus::DetailEvents => self.events_scroll = self.max_events_scroll(),
            Focus::DetailSummary => self.summary_scroll = self.max_summary_scroll(),
            Focus::DetailTasks => {
                if let Some(detail) = &self.run_detail
                    && !detail.tasks.is_empty()
                {
                    self.move_selected_run_task_to(detail.tasks.len() - 1);
                }
            }
            _ => {}
        }
    }

    fn move_selected_run_task(&mut self, delta: i32) {
        let Some(detail) = &self.run_detail else {
            return;
        };
        if detail.tasks.is_empty() {
            self.selected_run_task_index = 0;
            return;
        }
        let next = self.selected_run_task_index as i32 + delta;
        self.move_selected_run_task_to(next.clamp(0, detail.tasks.len() as i32 - 1) as usize);
    }

    fn move_selected_run_task_to(&mut self, index: usize) {
        self.selected_run_task_index = index;
        let _ = self.reload_selected_task_log();
    }

    fn adjust_log_scroll(&mut self, delta: i32) {
        if delta > 0 {
            for _ in 0..delta {
                self.log_scroll_state.scroll_down();
            }
        } else {
            for _ in 0..delta.unsigned_abs() {
                self.log_scroll_state.scroll_up();
            }
        }
    }

    fn adjust_launcher_log_scroll(&mut self, delta: i32) {
        if delta > 0 {
            for _ in 0..delta {
                self.launcher_log_scroll_state.scroll_down();
            }
        } else {
            for _ in 0..delta.unsigned_abs() {
                self.launcher_log_scroll_state.scroll_up();
            }
        }
    }

    fn max_events_scroll(&self) -> u16 {
        self.run_detail
            .as_ref()
            .map(|detail| detail.events.len().saturating_sub(1))
            .unwrap_or(0)
            .min(u16::MAX as usize) as u16
    }

    fn max_summary_scroll(&self) -> u16 {
        32
    }

    fn cycle_focus(&mut self, reverse: bool) {
        let order = match self.screen {
            Screen::Launcher if self.show_launcher_failure_output() => [
                Focus::LauncherTasks,
                Focus::LauncherDetails,
                Focus::LauncherFailureOutput,
            ]
            .as_slice(),
            Screen::Launcher => [Focus::LauncherTasks, Focus::LauncherDetails].as_slice(),
            Screen::RunHistory => [Focus::HistoryRuns, Focus::HistoryDetails].as_slice(),
            Screen::RunDetail => [
                Focus::DetailTasks,
                Focus::DetailOutput,
                Focus::DetailEvents,
                Focus::DetailSummary,
            ]
            .as_slice(),
        };
        let current_index = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next_index = if reverse {
            current_index.checked_sub(1).unwrap_or(order.len() - 1)
        } else {
            (current_index + 1) % order.len()
        };
        self.focus = order[next_index];
    }

    fn start_run(&mut self) -> NaoResult<()> {
        if self.active_run.is_some() {
            self.status_message = Some(SharedString::from("a run is already active"));
            return Ok(());
        }
        let goal_tasks = self.launcher_goal_tasks();
        if goal_tasks.is_empty() {
            self.status_message = Some(SharedString::from("no task is available to run"));
            return Ok(());
        }

        let plan = self.engine.plan_run(&self.recipe_path, &goal_tasks)?;
        self.spawn_run(plan)?;
        Ok(())
    }

    fn launcher_goal_tasks(&self) -> Vec<String> {
        if !self.selected_goals.is_empty() {
            return self
                .selected_goals
                .iter()
                .map(|task| task.as_str().to_owned())
                .collect::<Vec<_>>();
        }

        self.tasks
            .get(self.selected_task_index)
            .map(|task| vec![task.name.as_str().to_owned()])
            .unwrap_or_default()
    }

    fn spawn_run(&mut self, plan: PlannedRun) -> NaoResult<()> {
        let (sender, receiver) = mpsc::channel();
        let run_started_at = self.pal.now();
        let run_started_system_time = self.pal.system_time();
        let recipe_path = self.recipe_path.clone();
        let recipe_directory = recipe_path.parent().unwrap_or_else(|| FilePath::from("."));
        let run_directory = RunArtifactWriter::preview_run_directory(
            &recipe_directory,
            &plan
                .requested_tasks
                .iter()
                .map(|task| task.as_str().to_owned())
                .collect::<Vec<_>>(),
            run_started_system_time,
        );
        let pal = self.pal.clone();
        let worker_plan = plan.clone();

        std::thread::spawn(move || {
            let engine = RunEngine::new(pal);
            let result = engine.execute_planned_run_with_observer_started_at(
                &recipe_path,
                &worker_plan,
                &mut NoopObserver,
                run_started_at,
                run_started_system_time,
            );
            let _ = sender.send(result);
        });

        self.active_run = Some(ActiveRunHandle {
            run_directory: run_directory.clone(),
            receiver,
        });
        self.launched_run_in_session = true;
        self.status_message = Some(SharedString::from("run started"));
        self.open_run(&run_directory)?;
        Ok(())
    }

    fn open_run(&mut self, run_directory: &FilePath) -> NaoResult<()> {
        self.open_run_directory = Some(run_directory.clone());
        self.events_scroll = 0;
        self.summary_scroll = 0;
        self.refresh_open_run_detail()
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        match self.screen {
            Screen::Launcher => self.render_launcher(frame),
            Screen::RunHistory => self.render_history(frame),
            Screen::RunDetail => self.render_detail(frame),
        }
        if self.help_visible {
            self.render_help(frame);
        }
    }

    fn render_launcher(&mut self, frame: &mut Frame<'_>) {
        let layout = top_level_layout(frame.area());
        self.render_header(
            frame,
            layout[0],
            "Launch Tasks",
            self.status_message
                .as_ref()
                .map(|message| message.as_str())
                .unwrap_or(""),
        );
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body[0]);
        let right = if self.show_launcher_failure_output() {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(body[1])
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(100)])
                .split(body[1])
        };
        let items = self
            .tasks
            .iter()
            .map(|task| {
                let marker = if self.selected_goals.contains(task.name.as_str()) {
                    "[x]"
                } else {
                    "[ ]"
                };
                ListItem::new(Line::from(format!(
                    "{marker} {:<20} {}",
                    task.name.as_str(),
                    task.description.as_deref().unwrap_or("-")
                )))
            })
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        if !self.tasks.is_empty() {
            list_state.select(Some(self.selected_task_index));
        }
        let list = List::new(items)
            .block(focused_block(
                "Available Tasks",
                self.focus == Focus::LauncherTasks,
            ))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        StatefulWidget::render(list, left[0], frame.buffer_mut(), &mut list_state);

        let details_text = if let Some(task) = self.tasks.get(self.selected_task_index) {
            vec![
                Line::from(format!("name: {}", task.name.as_str())),
                Line::from(format!(
                    "deps: {}",
                    if task.dependencies.is_empty() {
                        "-".to_owned()
                    } else {
                        task.dependencies
                            .iter()
                            .map(|dependency| dependency.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )),
                Line::from(format!(
                    "description: {}",
                    task.description.as_deref().unwrap_or("-")
                )),
            ]
        } else {
            vec![Line::from("no tasks")]
        };
        let details = Paragraph::new(details_text)
            .block(focused_block(
                "Task Details",
                self.focus == Focus::LauncherDetails,
            ))
            .wrap(Wrap { trim: false });
        frame.render_widget(details, left[1]);
        self.render_launcher_progress(frame, right[0]);
        if self.show_launcher_failure_output() {
            self.render_launcher_failed_output(frame, right[1]);
        }

        let selected_goals = if self.selected_goals.is_empty() {
            "-".to_owned()
        } else {
            self.selected_goals
                .iter()
                .map(|goal| goal.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let footer_text = format!(
            "Selected goals: {selected_goals} | Space toggle | Enter start run | r history | o failed output | ? help | q quit"
        );
        self.render_footer(frame, layout[2], &footer_text);
    }

    fn render_launcher_progress(&self, frame: &mut Frame<'_>, area: Rect) {
        let (title, text) = if self.launched_run_in_session {
            ("Run Progress", self.render_launcher_progress_lines())
        } else {
            ("Help", self.render_launcher_help_lines())
        };

        frame.render_widget(
            Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title(title))
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_launcher_progress_lines(&self) -> Vec<Line<'static>> {
        if let Some(detail) = &self.run_detail {
            let mut lines = vec![
                Line::from(format!("run: {}", detail.run_id.as_str())),
                Line::from(format!("result: {}", detail.result.as_str())),
                Line::from(format!(
                    "goals: {}",
                    if detail.requested_tasks.is_empty() {
                        "-".to_owned()
                    } else {
                        detail
                            .requested_tasks
                            .iter()
                            .map(|task| task.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )),
                Line::from(""),
            ];
            lines.extend(detail.tasks.iter().map(|task| {
                let mut row = format!(
                    "{} {:<20}",
                    render_task_state_emoji(task.status.as_str(), self.spinner_frame),
                    task.name.as_str()
                );
                if let Some(duration_nanos) = task.duration_nanos {
                    row.push_str(&format!("  {}", pretty_duration(duration_nanos)));
                } else if task.status.as_str() == "running" {
                    row.push_str("  running");
                }
                if task.status.as_str() == "failed"
                    && let Some(exit_code) = task.exit_code
                {
                    row.push_str(&format!("  exit {exit_code}"));
                }
                if let Some(outcome_message) = &task.outcome_message {
                    row.push_str(&format!("  {}", outcome_message.as_str()));
                }
                Line::from(row)
            }));
            if detail.result.as_str() != "running"
                && let Some(duration_nanos) = detail.duration_nanos
            {
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "🏁 total {}",
                    pretty_duration(duration_nanos)
                )));
            }
            if detail.tasks.is_empty() {
                lines.push(Line::from("no tasks"));
            }
            lines
        } else {
            vec![Line::from("waiting for run data...")]
        }
    }

    fn render_launcher_help_lines(&self) -> Vec<Line<'static>> {
        vec![
            Line::from("  TUI usage"),
            Line::from(""),
            Line::from("  j / k: move through tasks"),
            Line::from("  Space: toggle explicit goal selection"),
            Line::from("  Enter: start selected task"),
            Line::from("  Tab: switch pane focus"),
            Line::from("  o: focus failed task output"),
            Line::from("  r: open run history"),
            Line::from("  2: open run detail"),
            Line::from("  ?: open help"),
            Line::from("  q: quit"),
        ]
    }

    fn render_launcher_failed_output(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let title = self
            .launcher_failed_task_name
            .as_ref()
            .map(|task_name| format!("Failed Task Output: {}", task_name.as_str()))
            .unwrap_or_else(|| "Failed Task Output".to_owned());
        Self::render_scrollable_output(
            frame,
            area,
            &title,
            self.focus == Focus::LauncherFailureOutput,
            &self.launcher_failed_task_log_lines,
            &mut self.launcher_log_scroll_state,
        );
    }

    fn render_history(&self, frame: &mut Frame<'_>) {
        let layout = top_level_layout(frame.area());
        self.render_header(frame, layout[0], "Run History", "");
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        let items = self
            .runs
            .iter()
            .map(|run| {
                ListItem::new(Line::from(format!(
                    "{}   {}",
                    run.run_id.as_str(),
                    run.result.as_str()
                )))
            })
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        if !self.runs.is_empty() {
            list_state.select(Some(self.selected_run_index));
        }
        let list = List::new(items)
            .block(focused_block("Runs", self.focus == Focus::HistoryRuns))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        StatefulWidget::render(list, body[0], frame.buffer_mut(), &mut list_state);

        let details_text = if let Some(run) = self.runs.get(self.selected_run_index) {
            vec![
                Line::from(format!(
                    "goals: {}",
                    run.requested_tasks
                        .iter()
                        .map(|task| task.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Line::from(format!("result: {}", run.result.as_str())),
                Line::from(format!("tasks: {}", run.task_count)),
                Line::from(format!(
                    "failure: {}",
                    run.failure_message.as_deref().unwrap_or("-")
                )),
            ]
        } else {
            vec![Line::from("no runs")]
        };
        frame.render_widget(
            Paragraph::new(details_text).block(focused_block(
                "Selected Run",
                self.focus == Focus::HistoryDetails,
            )),
            body[1],
        );
        self.render_footer(
            frame,
            layout[2],
            "Enter open run | l launcher | R refresh history | ? help | q quit",
        );
    }

    fn render_detail(&mut self, frame: &mut Frame<'_>) {
        let layout = top_level_layout(frame.area());
        let run_title = self
            .run_detail
            .as_ref()
            .map(|detail| detail.run_id.as_str())
            .unwrap_or("-");
        self.render_header(frame, layout[0], "Run Detail", &format!("run: {run_title}"));

        if frame.area().width < 100 {
            self.render_narrow_detail(frame, layout[1]);
        } else {
            self.render_wide_detail(frame, layout[1]);
        }

        self.render_footer(
            frame,
            layout[2],
            &format!(
                "Focus {} | Tab pane | j/k move | Enter follow task | t/o/e/s focus | r history | L auto-follow | ? help | q quit",
                self.focus_label()
            ),
        );
    }

    fn render_wide_detail(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[0]);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(rows[1]);
        self.render_detail_tasks(frame, top[0]);
        self.render_detail_output(frame, top[1]);
        self.render_detail_events(frame, bottom[0]);
        self.render_detail_summary(frame, bottom[1]);
    }

    fn render_narrow_detail(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);
        let tabs = Tabs::new(vec!["Tasks", "Log", "Events", "Summary"])
            .select(match self.focus {
                Focus::DetailTasks => 0,
                Focus::DetailOutput => 1,
                Focus::DetailEvents => 2,
                Focus::DetailSummary => 3,
                _ => 0,
            })
            .block(Block::default().borders(Borders::ALL).title("Panels"));
        frame.render_widget(tabs, layout[0]);
        match self.focus {
            Focus::DetailOutput => self.render_detail_output(frame, layout[1]),
            Focus::DetailEvents => self.render_detail_events(frame, layout[1]),
            Focus::DetailSummary => self.render_detail_summary(frame, layout[1]),
            _ => self.render_detail_tasks(frame, layout[1]),
        }
    }

    fn render_detail_tasks(&self, frame: &mut Frame<'_>, area: Rect) {
        let items = self
            .run_detail
            .as_ref()
            .map(|detail| {
                detail
                    .tasks
                    .iter()
                    .map(|task| {
                        ListItem::new(Line::from(format!(
                            "{:<20} {:<10} {}{}",
                            task.name.as_str(),
                            task.status.as_str(),
                            task.exit_code
                                .map(|code| format!("exit {code}"))
                                .unwrap_or_default(),
                            task.outcome_message
                                .as_ref()
                                .map(|message| format!(" outcome: {}", message.as_str()))
                                .unwrap_or_default(),
                        )))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(self.selected_run_task_index));
        }
        let list = List::new(items)
            .block(focused_block("Tasks", self.focus == Focus::DetailTasks))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        StatefulWidget::render(list, area, frame.buffer_mut(), &mut list_state);
    }

    fn render_detail_output(&mut self, frame: &mut Frame<'_>, area: Rect) {
        Self::render_scrollable_output(
            frame,
            area,
            "Task Output",
            self.focus == Focus::DetailOutput,
            &self.task_log_lines,
            &mut self.log_scroll_state,
        );
    }

    fn render_detail_events(&self, frame: &mut Frame<'_>, area: Rect) {
        let text = self
            .run_detail
            .as_ref()
            .map(|detail| {
                detail
                    .events
                    .iter()
                    .map(|event| {
                        let suffix = event
                            .task_name
                            .as_ref()
                            .map(|task| format!(" {}", task.as_str()))
                            .unwrap_or_default();
                        Line::from(format!(
                            "{} {}{}",
                            event.timestamp.as_str(),
                            event.event_type.as_str(),
                            suffix
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![Line::from("no events")]);
        frame.render_widget(
            Paragraph::new(text)
                .block(focused_block("Events", self.focus == Focus::DetailEvents))
                .scroll((self.events_scroll, 0))
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_detail_summary(&self, frame: &mut Frame<'_>, area: Rect) {
        let text = if let Some(detail) = &self.run_detail {
            vec![
                Line::from(format!(
                    "requested tasks: {}",
                    detail
                        .requested_tasks
                        .iter()
                        .map(|task| task.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Line::from(format!("result: {}", detail.result.as_str())),
                Line::from(format!(
                    "failure: {}",
                    detail.failure_message.as_deref().unwrap_or("-")
                )),
                Line::from(format!(
                    "selected task: {}",
                    detail
                        .tasks
                        .get(self.selected_run_task_index)
                        .map(|task| task.name.as_str())
                        .unwrap_or("-")
                )),
                Line::from(format!(
                    "selected outcome: {}",
                    detail
                        .tasks
                        .get(self.selected_run_task_index)
                        .and_then(|task| task.outcome_message.as_deref())
                        .unwrap_or("-")
                )),
                Line::from(format!(
                    "auto-follow: {}",
                    if self.auto_follow_log { "on" } else { "off" }
                )),
            ]
        } else {
            vec![Line::from("no run selected")]
        };
        frame.render_widget(
            Paragraph::new(text)
                .block(focused_block(
                    "Run Summary",
                    self.focus == Focus::DetailSummary,
                ))
                .scroll((self.summary_scroll, 0))
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_help(&self, frame: &mut Frame<'_>) {
        let help_area = centered_rect(frame.area(), 80, 60);
        frame.render_widget(Clear, help_area);
        let text = vec![
            Line::from("q close help or quit"),
            Line::from("? open help"),
            Line::from("1 launcher"),
            Line::from("2 run detail"),
            Line::from("3 run history"),
            Line::from("Tab / Shift-Tab cycle panes"),
            Line::from("Launcher: j/k move, Space toggle, Enter start run"),
            Line::from("History: j/k move, Enter open, R refresh"),
            Line::from("Detail: t/o/e/s focus, j/k move, g/G top/bottom, L auto-follow"),
        ];
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Help"))
                .wrap(Wrap { trim: false }),
            help_area,
        );
    }

    fn render_header(&self, frame: &mut Frame<'_>, area: Rect, title: &str, suffix: &str) {
        let header = if suffix.is_empty() {
            format!("nao TUI | screen: {title}")
        } else {
            format!("nao TUI | screen: {title} | {suffix}")
        };
        frame.render_widget(
            Paragraph::new(header).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect, text: &str) {
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn focus_label(&self) -> &'static str {
        match self.focus {
            Focus::LauncherTasks => "launcher tasks",
            Focus::LauncherDetails => "launcher details",
            Focus::LauncherFailureOutput => "launcher failed output",
            Focus::HistoryRuns => "history runs",
            Focus::HistoryDetails => "history details",
            Focus::DetailTasks => "detail tasks",
            Focus::DetailOutput => "task output",
            Focus::DetailEvents => "events",
            Focus::DetailSummary => "summary",
        }
    }

    fn show_launcher_failure_output(&self) -> bool {
        self.run_detail
            .as_ref()
            .map(|detail| {
                detail.result.as_str() == "failed" && self.launcher_failed_task_name.is_some()
            })
            .unwrap_or(false)
    }

    fn render_scrollable_output(
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        focused: bool,
        lines: &[SharedString],
        scroll_state: &mut ScrollViewState,
    ) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let content_lines = if lines.is_empty() {
            vec![Line::from("no task output yet")]
        } else {
            lines
                .iter()
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>()
        };
        let content_width = content_lines
            .iter()
            .map(|line| line.width() as u16)
            .max()
            .unwrap_or(1);
        let content_height = content_lines.len().min(u16::MAX as usize) as u16;
        let inner_area = focused_block(title, focused).inner(area);
        let visible_height = inner_area.height.max(1);
        let max_vertical_offset = content_height.saturating_sub(visible_height);
        let current_offset = scroll_state.offset();
        scroll_state.set_offset(Position::new(
            current_offset.x,
            current_offset.y.min(max_vertical_offset),
        ));

        let content_size = Size::new(content_width.max(inner_area.width), content_height.max(1));
        let mut scroll_view =
            ScrollView::new(content_size).scrollbars_visibility(ScrollbarVisibility::Automatic);
        scroll_view.render_widget(
            Paragraph::new(content_lines).wrap(Wrap { trim: false }),
            Rect::new(0, 0, content_size.width, content_size.height),
        );
        frame.render_widget(focused_block(title, focused), area);
        frame.render_stateful_widget(scroll_view, inner_area, scroll_state);
    }
}

#[derive(Debug)]
struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> NaoResult<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

struct NoopObserver;

impl nao_engine::RunObserver for NoopObserver {}

fn top_level_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area)
        .to_vec()
}

fn focused_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let block = Block::default().borders(Borders::ALL).title(title);
    if focused {
        block.border_style(Style::default().add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn adjust_scroll(current: u16, delta: i32, max: u16) -> u16 {
    let next = current as i32 + delta;
    next.clamp(0, max as i32) as u16
}

fn spinner_frames() -> &'static [&'static str] {
    &["⠋ ", "⠙ ", "⠹ ", "⠸ ", "⠼ ", "⠴ ", "⠦ ", "⠧ ", "⠇ ", "⠏ "]
}

fn render_task_state_emoji(status: &str, spinner_frame: usize) -> &'static str {
    match status {
        "pending" => "⚪",
        "running" => spinner_frames()[spinner_frame % spinner_frames().len()],
        "completed" => "✅",
        "failed" => "❌",
        "skipped" => "⏭ ",
        _ => "⚪",
    }
}

fn pretty_duration(duration_nanos: u128) -> String {
    if duration_nanos < 1_000 {
        return format!("{duration_nanos}ns");
    }
    if duration_nanos < 1_000_000 {
        return format!("{:.1}us", duration_nanos as f64 / 1_000.0);
    }
    if duration_nanos < 1_000_000_000 {
        return format!("{:.1}ms", duration_nanos as f64 / 1_000_000.0);
    }
    format!("{:.1}s", duration_nanos as f64 / 1_000_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::{App, Focus, Screen, pretty_duration, render_task_state_emoji};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_base::timestamp::Timestamp;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use nao_pal::process_command::ProcessCommand;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_result::ProcessResult;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
    use std::time::SystemTime;

    fn test_app() -> App {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }

              task "test" description="Test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        pal.set_process_execution(
            ProcessCommand {
                executable: "./scripts/build.sh".into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(1),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(2),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(3),
                    exit_code: Some(0),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(3),
                exit_code: Some(0),
            },
        );
        pal.set_process_execution(
            ProcessCommand {
                executable: "./scripts/test.sh".into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(4),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(5),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(6),
                    exit_code: Some(0),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(3),
                finished_at: Timestamp::new(6),
                exit_code: Some(0),
            },
        );
        App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap()
    }

    #[test]
    fn launcher_keys_toggle_goals_and_switch_screens() {
        let mut app = test_app();
        app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        app.handle_key_event(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .unwrap();
        app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.screen, Screen::RunHistory);
        assert_eq!(app.focus, Focus::HistoryRuns);
        assert!(app.selected_goals.contains("test"));
    }

    #[test]
    fn launcher_defaults_run_target_to_selected_task() {
        let mut app = test_app();
        app.selected_task_index = 1;

        assert_eq!(app.launcher_goal_tasks(), vec!["test".to_owned()]);
    }

    #[test]
    fn launcher_prefers_explicit_goal_selection() {
        let mut app = test_app();
        app.selected_task_index = 0;
        app.selected_goals.insert("test".into());

        assert_eq!(app.launcher_goal_tasks(), vec!["test".to_owned()]);
    }

    #[test]
    fn launching_keeps_the_launcher_screen_active() {
        let mut app = test_app();
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.screen, Screen::Launcher);
        assert!(app.active_run.is_some());
    }

    #[test]
    fn tab_cycles_run_detail_focus() {
        let mut app = test_app();
        app.screen = Screen::RunDetail;
        app.focus = Focus::DetailTasks;

        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.focus, Focus::DetailOutput);

        app.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(app.focus, Focus::DetailTasks);
    }

    #[test]
    fn hotkey_two_defaults_run_detail_to_output_focus() {
        let mut app = test_app();

        app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.screen, Screen::RunDetail);
        assert_eq!(app.focus, Focus::DetailOutput);
    }

    #[test]
    fn launcher_shows_failed_task_output_for_failed_runs() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }
            }
            "#,
        );
        pal.set_file(
            ".nao/runs/2026-03-20T10-00-00Z-build/nao-summary.json",
            r#"{
              "result":"failed",
              "failure_message":"boom",
              "run":{"requested_tasks":["build"],"duration_nanos":"10"},
              "tasks":[
                {
                  "name":"build",
                  "status":"failed",
                  "result":"failed",
                  "exit_code":1,
                  "outcome_message":"12 files checked",
                  "duration_nanos":"10",
                  "log_file":"build.log"
                }
              ]
            }"#,
        );
        pal.set_file(
            ".nao/runs/2026-03-20T10-00-00Z-build/build.log",
            "[2026-03-20T10:00:01Z] stderr: compile failed\n",
        );

        let mut app = App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap();
        app.open_run(&FilePath::from(".nao/runs/2026-03-20T10-00-00Z-build"))
            .unwrap();

        assert!(app.show_launcher_failure_output());
        assert_eq!(app.launcher_failed_task_name.as_deref(), Some("build"));
        assert_eq!(
            app.launcher_failed_task_log_lines,
            vec![SharedString::from("compile failed")]
        );
        assert_eq!(
            app.run_detail
                .as_ref()
                .and_then(|detail| detail.tasks.first())
                .and_then(|task| task.outcome_message.as_ref())
                .map(|value| value.as_str()),
            Some("12 files checked")
        );
    }

    #[test]
    fn launcher_progress_includes_task_outcomes() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "test" description="Test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        pal.set_file(
            ".nao/runs/2026-03-20T10-00-00Z-test/nao-summary.json",
            r#"{
              "result":"completed",
              "failure_message":null,
              "run":{"requested_tasks":["test"],"duration_nanos":"10"},
              "tasks":[
                {
                  "name":"test",
                  "status":"completed",
                  "result":"success",
                  "exit_code":0,
                  "outcome_message":"30 tests passed",
                  "duration_nanos":"10",
                  "log_file":"test.log"
                }
              ]
            }"#,
        );

        let mut app = App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap();
        app.launched_run_in_session = true;
        app.open_run(&FilePath::from(".nao/runs/2026-03-20T10-00-00Z-test"))
            .unwrap();

        let rendered = app
            .render_launcher_progress_lines()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("30 tests passed"));
    }

    #[test]
    fn renders_task_state_emojis() {
        assert_eq!(render_task_state_emoji("pending", 0), "⚪");
        assert_eq!(render_task_state_emoji("running", 0), "⠋ ");
        assert_eq!(render_task_state_emoji("running", 1), "⠙ ");
        assert_eq!(render_task_state_emoji("completed", 0), "✅");
        assert_eq!(render_task_state_emoji("failed", 0), "❌");
        assert_eq!(render_task_state_emoji("skipped", 0), "⏭ ");
    }

    #[test]
    fn formats_durations_for_progress_rows() {
        assert_eq!(pretty_duration(999), "999ns");
        assert_eq!(pretty_duration(1_200), "1.2us");
        assert_eq!(pretty_duration(2_500_000), "2.5ms");
        assert_eq!(pretty_duration(2_000_000_000), "2.0s");
    }
}
