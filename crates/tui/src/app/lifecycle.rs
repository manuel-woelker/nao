use super::ActiveRunHandle;
use super::App;
use super::OpenRunRefreshState;
use super::RunDetailRefreshOutcome;
use crate::artifact_store::{discover_runs, load_run_detail, load_task_log_lines};
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_engine::PlannedRun;
use nao_engine::RunEngine;
use nao_engine::run_artifact_writer::RunArtifactWriter;
use std::sync::mpsc;

impl App {
    pub(super) fn refresh(&mut self) -> NaoResult<()> {
        self.refresh_tick = self.refresh_tick.wrapping_add(1);
        self.spinner_frame = (self.spinner_frame + 1) % super::spinner_frames().len();
        self.refresh_active_run()?;
        self.refresh_open_run_artifacts()?;
        Ok(())
    }

    fn refresh_open_run_artifacts(&mut self) -> NaoResult<()> {
        if self.should_reload_open_run_detail() {
            let outcome = self.reload_open_run_detail()?;
            if outcome.selected_task_changed {
                self.open_run_refresh_state.force_selected_task_log_reload = true;
            }
            if outcome.failed_task_changed {
                self.open_run_refresh_state.force_launcher_failed_log_reload = true;
            }
        }
        if self.should_reload_selected_task_log() {
            self.reload_selected_task_log()?;
            self.open_run_refresh_state.force_selected_task_log_reload = false;
        }
        if self.should_reload_launcher_failed_task_log() {
            self.reload_launcher_failed_task_log()?;
            self.open_run_refresh_state.force_launcher_failed_log_reload = false;
        }
        Ok(())
    }

    fn refresh_active_run(&mut self) -> NaoResult<()> {
        let Some(active_run) = &self.active_run else {
            return Ok(());
        };
        match active_run.receiver.try_recv() {
            Ok(Ok(result)) => {
                self.status_message = Some(SharedString::from("run completed"));
                let completed_run_directory = result.run_directory.clone();
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

    fn should_reload_open_run_detail(&self) -> bool {
        if self.open_run_refresh_state.force_detail_reload {
            return true;
        }
        if !self.is_open_run_active() {
            return false;
        }

        self.refresh_tick.is_multiple_of(4)
    }

    fn should_reload_selected_task_log(&self) -> bool {
        if self.open_run_refresh_state.force_selected_task_log_reload {
            return true;
        }
        if !self.is_open_run_active() || !self.auto_follow_log || !self.show_detail_selected_task()
        {
            return false;
        }

        self.selected_task_is_active() && self.refresh_tick.is_multiple_of(2)
    }

    fn should_reload_launcher_failed_task_log(&self) -> bool {
        if self.open_run_refresh_state.force_launcher_failed_log_reload {
            return true;
        }
        if !self.is_open_run_active() {
            return false;
        }

        self.show_launcher_failure_output() && self.refresh_tick.is_multiple_of(2)
    }

    fn is_open_run_active(&self) -> bool {
        let Some(open_run_directory) = &self.open_run_directory else {
            return false;
        };
        self.active_run
            .as_ref()
            .is_some_and(|active_run| active_run.run_directory == *open_run_directory)
    }

    fn show_detail_selected_task(&self) -> bool {
        self.run_detail
            .as_ref()
            .and_then(|detail| detail.tasks.get(self.selected_run_task_index))
            .is_some()
    }

    fn selected_task_is_active(&self) -> bool {
        self.run_detail
            .as_ref()
            .and_then(|detail| detail.tasks.get(self.selected_run_task_index))
            .is_some_and(|task| matches!(task.status.as_str(), "running" | "failed"))
    }

    pub(super) fn reload_history(&mut self) -> NaoResult<()> {
        self.runs = discover_runs(&*self.pal, &self.recipe_path)?;
        if self.selected_run_index >= self.runs.len() && !self.runs.is_empty() {
            self.selected_run_index = self.runs.len() - 1;
        }
        Ok(())
    }

    fn reload_open_run_detail(&mut self) -> NaoResult<RunDetailRefreshOutcome> {
        let Some(run_directory) = &self.open_run_directory else {
            self.run_detail = None;
            return Ok(RunDetailRefreshOutcome {
                selected_task_changed: false,
                failed_task_changed: false,
            });
        };
        let previous_selected_log_file = self
            .run_detail
            .as_ref()
            .and_then(|detail| detail.tasks.get(self.selected_run_task_index))
            .map(|task| task.log_file.clone());
        let previous_failed_log_file = self
            .run_detail
            .as_ref()
            .and_then(|detail| {
                detail
                    .tasks
                    .iter()
                    .find(|task| task.status.as_str() == "failed")
            })
            .map(|task| task.log_file.clone());
        self.run_detail = Some(load_run_detail(&*self.pal, run_directory)?);
        if let Some(detail) = &self.run_detail {
            if detail.tasks.is_empty() {
                self.selected_run_task_index = 0;
            } else {
                self.selected_run_task_index = self
                    .selected_run_task_index
                    .min(detail.tasks.len().saturating_sub(1));
            }
        }
        self.open_run_refresh_state.force_detail_reload = false;
        let selected_task_changed = self
            .run_detail
            .as_ref()
            .and_then(|detail| detail.tasks.get(self.selected_run_task_index))
            .map(|task| task.log_file.clone())
            != previous_selected_log_file;
        let failed_task_changed = self
            .run_detail
            .as_ref()
            .and_then(|detail| {
                detail
                    .tasks
                    .iter()
                    .find(|task| task.status.as_str() == "failed")
            })
            .map(|task| task.log_file.clone())
            != previous_failed_log_file;

        Ok(RunDetailRefreshOutcome {
            selected_task_changed,
            failed_task_changed,
        })
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

    pub(super) fn reload_selected_task_log(&mut self) -> NaoResult<()> {
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

    pub(super) fn start_run(&mut self) -> NaoResult<()> {
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

    pub(super) fn launcher_goal_tasks(&self) -> Vec<String> {
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
        let run_directory = RunArtifactWriter::preview_run_directory(
            &recipe_path,
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

    pub(super) fn open_run(&mut self, run_directory: &FilePath) -> NaoResult<()> {
        self.open_run_directory = Some(run_directory.clone());
        self.open_run_refresh_state = OpenRunRefreshState {
            force_detail_reload: true,
            force_selected_task_log_reload: true,
            force_launcher_failed_log_reload: true,
        };
        self.events_scroll = 0;
        self.summary_scroll = 0;
        self.refresh_open_run_artifacts()
    }
}

struct NoopObserver;

impl nao_engine::RunObserver for NoopObserver {}
