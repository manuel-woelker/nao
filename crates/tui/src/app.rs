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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, StatefulWidget, Tabs, Wrap,
};
use std::collections::BTreeSet;
use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

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
    selected_run_task_index: usize,
    task_log_lines: Vec<SharedString>,
    log_scroll: u16,
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
            selected_run_task_index: 0,
            task_log_lines: Vec::new(),
            log_scroll: 0,
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
            self.log_scroll = self.max_log_scroll();
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
                self.focus = Focus::DetailTasks;
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
            KeyCode::Down | KeyCode::Char('j') => self.move_launcher_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_launcher_selection(-1),
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

    fn handle_history_key(&mut self, key_event: KeyEvent) -> NaoResult<()> {
        match key_event.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_history_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_history_selection(-1),
            KeyCode::Enter => {
                if let Some(run) = self.runs.get(self.selected_run_index) {
                    let run_directory = run.run_directory.clone();
                    self.open_run(&run_directory)?;
                    self.screen = Screen::RunDetail;
                    self.focus = Focus::DetailTasks;
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
                self.log_scroll = adjust_scroll(self.log_scroll, delta, self.max_log_scroll())
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
            Focus::DetailOutput => self.log_scroll = 0,
            Focus::DetailEvents => self.events_scroll = 0,
            Focus::DetailSummary => self.summary_scroll = 0,
            Focus::DetailTasks => self.move_selected_run_task_to(0),
            _ => {}
        }
    }

    fn scroll_current_pane_to_bottom(&mut self) {
        match self.focus {
            Focus::DetailOutput => self.log_scroll = self.max_log_scroll(),
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

    fn max_log_scroll(&self) -> u16 {
        self.task_log_lines
            .len()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
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

    fn render(&self, frame: &mut Frame<'_>) {
        match self.screen {
            Screen::Launcher => self.render_launcher(frame),
            Screen::RunHistory => self.render_history(frame),
            Screen::RunDetail => self.render_detail(frame),
        }
        if self.help_visible {
            self.render_help(frame);
        }
    }

    fn render_launcher(&self, frame: &mut Frame<'_>) {
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
        self.render_launcher_progress(frame, body[1]);

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
            "Selected goals: {selected_goals} | Space toggle | Enter start run | r history | ? help | q quit"
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
            Line::from("  r: open run history"),
            Line::from("  2: open run detail"),
            Line::from("  ?: open help"),
            Line::from("  q: quit"),
        ]
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

    fn render_detail(&self, frame: &mut Frame<'_>) {
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
            "Tab pane | j/k move | Enter follow task | t/o/e/s focus | r history | L auto-follow | ? help | q quit",
        );
    }

    fn render_wide_detail(&self, frame: &mut Frame<'_>, area: Rect) {
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

    fn render_narrow_detail(&self, frame: &mut Frame<'_>, area: Rect) {
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
                            "{:<20} {:<10} {}",
                            task.name.as_str(),
                            task.status.as_str(),
                            task.exit_code
                                .map(|code| format!("exit {code}"))
                                .unwrap_or_default()
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

    fn render_detail_output(&self, frame: &mut Frame<'_>, area: Rect) {
        let text = if self.task_log_lines.is_empty() {
            vec![Line::from("no task output yet")]
        } else {
            self.task_log_lines
                .iter()
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(text)
                .block(focused_block(
                    "Task Output",
                    self.focus == Focus::DetailOutput,
                ))
                .scroll((
                    clamp_scroll_for_viewport(
                        self.log_scroll,
                        self.task_log_lines.len(),
                        area.height,
                    ),
                    0,
                ))
                .wrap(Wrap { trim: false }),
            area,
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

fn clamp_scroll_for_viewport(current: u16, line_count: usize, area_height: u16) -> u16 {
    let visible_lines = area_height.saturating_sub(2) as usize;
    let max_scroll = line_count
        .saturating_sub(visible_lines)
        .min(u16::MAX as usize) as u16;
    current.min(max_scroll)
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
