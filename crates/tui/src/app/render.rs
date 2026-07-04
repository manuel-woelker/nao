use super::App;
use super::Focus;
use super::Screen;
use crate::artifact_store::RunSummaryRecord;
use nao_base::shared_string::SharedString;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, StatefulWidget, Tabs, Wrap,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

impl App {
    pub(super) fn render(&mut self, frame: &mut Frame<'_>) {
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
        let layout = super::top_level_layout(frame.area());
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
            .block(super::focused_block(
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
            .block(super::focused_block(
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

    pub(super) fn render_launcher_progress_lines(&self) -> Vec<Line<'static>> {
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
                    super::render_task_state_emoji(task.status.as_str(), self.spinner_frame),
                    task.name.as_str()
                );
                if let Some(duration_nanos) = task.duration_nanos {
                    row.push_str(&format!("  {}", super::pretty_duration(duration_nanos)));
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
                } else if let Some(status_message) = &task.status_message {
                    row.push_str(&format!("  {}", status_message.as_str()));
                }
                Line::from(row)
            }));
            if detail.result.as_str() != "running"
                && let Some(duration_nanos) = detail.duration_nanos
            {
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "🏁 total {}",
                    super::pretty_duration(duration_nanos)
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
        let layout = super::top_level_layout(frame.area());
        self.render_header(frame, layout[0], "Run History", "");
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        let items = self
            .runs
            .iter()
            .map(render_history_list_item)
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        if !self.runs.is_empty() {
            list_state.select(Some(self.selected_run_index));
        }
        let list = List::new(items)
            .block(super::focused_block(
                "Runs",
                self.focus == Focus::HistoryRuns,
            ))
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
            Paragraph::new(details_text).block(super::focused_block(
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
        let layout = super::top_level_layout(frame.area());
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
            .block(super::focused_block(
                "Tasks",
                self.focus == Focus::DetailTasks,
            ))
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
                .block(super::focused_block(
                    "Events",
                    self.focus == Focus::DetailEvents,
                ))
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
                .block(super::focused_block(
                    "Run Summary",
                    self.focus == Focus::DetailSummary,
                ))
                .scroll((self.summary_scroll, 0))
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_help(&self, frame: &mut Frame<'_>) {
        let help_area = super::centered_rect(frame.area(), 80, 60);
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
        let inner_area = super::focused_block(title, focused).inner(area);
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
        frame.render_widget(super::focused_block(title, focused), area);
        frame.render_stateful_widget(scroll_view, inner_area, scroll_state);
    }
}

fn render_history_list_item(run: &RunSummaryRecord) -> ListItem<'static> {
    ListItem::new(Line::from(format!(
        "{}   {}",
        run.run_id.as_str(),
        run.result.as_str()
    )))
}
