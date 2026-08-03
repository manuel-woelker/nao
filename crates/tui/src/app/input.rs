use super::App;
use super::Focus;
use super::Screen;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nao_base::result::NaoResult;

impl App {
    pub(super) fn handle_key_event(&mut self, key_event: KeyEvent) -> NaoResult<bool> {
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
            KeyCode::Char('r') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.restart_run()?;
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
                    super::adjust_scroll(self.events_scroll, delta, self.max_events_scroll());
            }
            Focus::DetailSummary => {
                self.summary_scroll =
                    super::adjust_scroll(self.summary_scroll, delta, self.max_summary_scroll());
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
        if self.selected_run_task_index == index {
            return;
        }
        self.selected_run_task_index = index;
        self.open_run_refresh_state.force_selected_task_log_reload = true;
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

    pub(super) fn focus_label(&self) -> &'static str {
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

    pub(super) fn show_launcher_failure_output(&self) -> bool {
        self.run_detail
            .as_ref()
            .map(|detail| {
                detail.result.as_str() == "failed" && self.launcher_failed_task_name.is_some()
            })
            .unwrap_or(false)
    }
}
