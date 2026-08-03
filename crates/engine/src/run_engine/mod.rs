mod execution;
mod process_command;
mod selectors;

#[cfg(test)]
mod tests;

use crate::planned_run::PlannedRun;
use crate::run_artifact_writer::RunArtifactWriter;
use crate::run_artifact_writer::TaskArtifactRecord;
use crate::run_artifact_writer::task_log_file_name;
use crate::run_execution_result::RunExecutionResult;
use crate::run_execution_result::RunStatus;
use crate::run_execution_result::RunTaskResult;
use crate::run_observer::RunObserver;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::cancellation_token::CancellationToken;
use nao_pal::pal::PalHandle;
use nao_pal::process_output_stream::ProcessOutputStream;
use nao_recipe::{Task, TaskName, load_recipe_with_pal};
use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

const TASK_OUTCOME_PREFIX: &str = "Task outcome: ";
pub(crate) const TASK_STATUS_PREFIX: &str = "Task status: ";

/// Loads recipes, plans runs, and executes tasks.
pub struct RunEngine {
    pal: PalHandle,
}

type TaskLogLines = Vec<(Timestamp, ProcessOutputStream, String)>;
type TaskExecutionResult = Result<nao_pal::process_result::ProcessResult, SharedString>;
type TaskRunArtifacts = (
    SharedString,
    Vec<TaskArtifactRecord>,
    Vec<crate::task_event_record::TaskEventRecord>,
    RunStatus,
    Option<String>,
);
pub(crate) enum TaskExecutionMessage {
    Status {
        task_index: usize,
        message: SharedString,
    },
    OutputLine {
        task_index: usize,
        stream: ProcessOutputStream,
        line: SharedString,
    },
    Finished {
        task_index: usize,
        output: SharedString,
        log_lines: TaskLogLines,
        result: TaskExecutionResult,
    },
}

impl RunEngine {
    /// Creates a new run engine for the provided platform abstraction.
    pub fn new(pal: PalHandle) -> Self {
        Self { pal }
    }

    /// Lists every task declared in the recipe.
    pub fn list_tasks(&self, recipe_path: &FilePath) -> NaoResult<Vec<Task>> {
        Ok(load_recipe_with_pal(&*self.pal, recipe_path)?.tasks)
    }

    /// Plans a run for the requested top-level task names.
    pub fn plan_run(&self, recipe_path: &FilePath, task_names: &[String]) -> NaoResult<PlannedRun> {
        let selector_parts = selectors::split_task_selectors(task_names);
        if selector_parts.is_empty() {
            return Err(err!("usage: nao [--list] [--config <path>] [task-name...]"));
        }

        let recipe = load_recipe_with_pal(&*self.pal, recipe_path)?;
        let task_names = selectors::expand_task_selectors(&recipe.tasks, &selector_parts)?;
        let mut requested_tasks = Vec::with_capacity(task_names.len());
        let mut tasks = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let task_index = recipe
            .tasks
            .iter()
            .map(|task| (task.name.as_str(), task))
            .collect::<BTreeMap<_, _>>();

        for task_name in task_names {
            requested_tasks.push(TaskName::from(task_name.as_str()));
            self.collect_task(
                &task_index,
                &task_name,
                &mut visiting,
                &mut visited,
                &mut tasks,
            )?;
        }

        Ok(PlannedRun {
            requested_tasks,
            live_display: recipe.config.live_display,
            failure_mode: recipe.config.failure_mode,
            max_parallel_tasks: recipe.config.max_parallel_tasks.unwrap_or(1),
            tasks,
        })
    }

    /// Executes the planned tasks sequentially and returns rendered output.
    pub fn execute_run(
        &self,
        recipe_path: &FilePath,
        task_names: &[String],
    ) -> NaoResult<RunExecutionResult> {
        let run_started_at = self.pal.now();
        let run_started_system_time = self.pal.system_time();
        let plan = self.plan_run(recipe_path, task_names)?;
        let mut observer = execution::NoopRunObserver;
        self.execute_planned_run_with_observer_started_at(
            recipe_path,
            &plan,
            &mut observer,
            run_started_at,
            run_started_system_time,
        )
    }

    /// Executes an already planned run sequentially and returns rendered output.
    pub fn execute_planned_run(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
    ) -> NaoResult<RunExecutionResult> {
        let mut observer = execution::NoopRunObserver;
        self.execute_planned_run_with_observer_started_at(
            recipe_path,
            plan,
            &mut observer,
            self.pal.now(),
            self.pal.system_time(),
        )
    }

    /// Executes an already planned run sequentially and emits task lifecycle updates.
    pub fn execute_planned_run_with_observer(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
    ) -> NaoResult<RunExecutionResult> {
        self.execute_planned_run_with_observer_started_at(
            recipe_path,
            plan,
            observer,
            self.pal.now(),
            self.pal.system_time(),
        )
    }

    /// Executes an already planned run using the provided run start time and emits task lifecycle updates.
    pub fn execute_planned_run_with_observer_started_at(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
        run_started_at: Timestamp,
        run_started_system_time: SystemTime,
    ) -> NaoResult<RunExecutionResult> {
        self.execute_planned_run_with_observer_started_at_cancellable(
            recipe_path,
            plan,
            observer,
            run_started_at,
            run_started_system_time,
            &CancellationToken::new(),
        )
    }

    /// Executes an already planned run using the provided run start time and emits task lifecycle updates.
    pub fn execute_planned_run_with_observer_started_at_cancellable(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
        run_started_at: Timestamp,
        run_started_system_time: SystemTime,
        cancellation_token: &CancellationToken,
    ) -> NaoResult<RunExecutionResult> {
        let writer = RunArtifactWriter::new(
            self.pal.clone(),
            recipe_path,
            &plan
                .requested_tasks
                .iter()
                .map(|task| task.as_str().to_owned())
                .collect::<Vec<_>>(),
            run_started_at,
            run_started_system_time,
        )?;
        writer.write_plan(plan)?;
        writer.write_run_started(plan)?;
        let (output, task_records, _task_events, run_status, failure_message) = self
            .execute_planned_run_with_scheduler(
                recipe_path,
                plan,
                observer,
                run_started_at,
                &writer,
                cancellation_token,
            )?;

        let run_finished_at = self.pal.now();
        let overall_result = if matches!(run_status, RunStatus::Failed(_)) {
            "failed"
        } else {
            "completed"
        };
        writer.write_completion(
            plan,
            &task_records,
            run_finished_at,
            overall_result,
            failure_message.as_deref(),
        )?;

        Ok(RunExecutionResult {
            output,
            goal_tasks: plan
                .requested_tasks
                .iter()
                .map(|task| task.0.clone())
                .collect(),
            total_task_count: plan.tasks.len(),
            duration_nanos: run_finished_at
                .as_nanos()
                .saturating_sub(run_started_at.as_nanos()),
            run_directory: writer.run_directory(),
            task_results: task_records
                .iter()
                .map(|task_record| RunTaskResult {
                    name: task_record.name.clone(),
                    status: task_record.status.clone(),
                    result: task_record.result.clone(),
                    exit_code: task_record.exit_code,
                    duration_nanos: task_record.started_at.zip(task_record.finished_at).map(
                        |(started_at, finished_at)| {
                            finished_at.as_nanos().saturating_sub(started_at.as_nanos())
                        },
                    ),
                    outcome_message: task_record.outcome_message.clone(),
                    log_path: writer
                        .run_directory()
                        .join(task_log_file_name(task_record.name.as_str())),
                })
                .collect(),
            goal_outcome_message: execution::goal_outcome_message(
                &plan.requested_tasks,
                &task_records,
            ),
            status: run_status,
        })
    }

    fn collect_task<'task>(
        &self,
        task_index: &BTreeMap<&'task str, &'task Task>,
        task_name: &str,
        visiting: &mut BTreeSet<SharedString>,
        visited: &mut BTreeSet<SharedString>,
        tasks: &mut Vec<Task>,
    ) -> NaoResult<()> {
        if visited.contains(task_name) {
            return Ok(());
        }

        let visiting_name = SharedString::from(task_name);
        if !visiting.insert(visiting_name.clone()) {
            return Err(err!("task dependency cycle detected at `{task_name}`"));
        }

        let task = task_index
            .get(task_name)
            .copied()
            .ok_or_else(|| err!("task `{task_name}` not found"))?;

        for dependency in &task.dependencies {
            self.collect_task(task_index, dependency.as_str(), visiting, visited, tasks)?;
        }

        visiting.remove(&visiting_name);
        visited.insert(visiting_name);
        tasks.push(task.clone());
        Ok(())
    }
}
