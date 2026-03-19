use crate::planned_run::PlannedRun;
use crate::run_artifact_writer::RunArtifactWriter;
use crate::run_artifact_writer::TaskArtifactRecord;
use crate::run_execution_result::RunExecutionResult;
use crate::run_execution_result::RunStatus;
use crate::run_execution_result::TaskFailure;
use crate::run_observer::RunObserver;
use crate::task_event_record::TaskEventRecord;
use crate::task_output_framer::TaskOutputFramer;
use crate::task_run_state::TaskRunState;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::pal::PalHandle;
use nao_pal::process_command::ProcessCommand;
use nao_pal::process_environment_variable::ProcessEnvironmentVariable;
use nao_pal::process_output_stream::ProcessOutputStream;
use nao_recipe::{RunSpec, Task, TaskName, load_recipe_with_pal};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc;
use std::thread;

/// Loads recipes, plans runs, and executes tasks.
pub struct RunEngine {
    pal: PalHandle,
}

type TaskLogLines = Vec<(Timestamp, ProcessOutputStream, String)>;
type TaskExecutionResult = Result<nao_pal::process_result::ProcessResult, SharedString>;
type TaskRunArtifacts = (
    SharedString,
    Vec<TaskArtifactRecord>,
    Vec<TaskEventRecord>,
    RunStatus,
    Option<String>,
);
type TaskExecutionMessage = (usize, SharedString, TaskLogLines, TaskExecutionResult);

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
        let selector_parts = split_task_selectors(task_names);
        if selector_parts.is_empty() {
            return Err(err!("usage: nao [--list] [--config <path>] [task-name...]"));
        }

        let recipe = load_recipe_with_pal(&*self.pal, recipe_path)?;
        let task_names = expand_task_selectors(&recipe.tasks, &selector_parts)?;
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
        let plan = self.plan_run(recipe_path, task_names)?;
        self.execute_planned_run(recipe_path, &plan)
    }

    /// Executes an already planned run sequentially and returns rendered output.
    pub fn execute_planned_run(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
    ) -> NaoResult<RunExecutionResult> {
        let mut observer = NoopRunObserver;
        self.execute_planned_run_with_observer(recipe_path, plan, &mut observer)
    }

    /// Executes an already planned run sequentially and emits task lifecycle updates.
    pub fn execute_planned_run_with_observer(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
    ) -> NaoResult<RunExecutionResult> {
        let recipe_directory = recipe_path.parent().unwrap_or_else(|| FilePath::from("."));
        let recipe_directory = if recipe_directory.as_str().is_empty() {
            FilePath::from(".")
        } else {
            recipe_directory
        };
        let run_started_at = self.pal.now();
        let run_started_system_time = self.pal.system_time();
        let writer = RunArtifactWriter::new(
            self.pal.clone(),
            &recipe_directory,
            &plan
                .requested_tasks
                .iter()
                .map(|task| task.as_str().to_owned())
                .collect::<Vec<_>>(),
            run_started_at,
            run_started_system_time,
        );
        writer.write_plan(plan)?;
        let (output, task_records, task_events, run_status, failure_message) = if plan
            .max_parallel_tasks
            <= 1
        {
            self.execute_planned_run_sequential(&recipe_directory, plan, observer, run_started_at)?
        } else {
            self.execute_planned_run_concurrent(&recipe_directory, plan, observer, run_started_at)?
        };

        let run_finished_at = self.pal.now();
        let overall_result = if matches!(run_status, RunStatus::Failed(_)) {
            "failed"
        } else {
            "completed"
        };
        writer.write_completion(
            plan,
            &task_records,
            &task_events,
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
            status: run_status,
        })
    }

    fn execute_planned_run_sequential(
        &self,
        recipe_directory: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
        run_started_at: Timestamp,
    ) -> NaoResult<TaskRunArtifacts> {
        let mut output = SharedString::empty();
        let mut task_records = Vec::with_capacity(plan.tasks.len());
        let mut task_events = Vec::new();
        let mut failure_message = None;
        let mut failed = false;
        let mut successful_task_count = 0usize;
        let mut run_status = RunStatus::Completed;

        for task in &plan.tasks {
            if failed {
                observer.on_task_skipped(task.name.as_str());
                let skipped_at = self.pal.now();
                task_events.push(TaskEventRecord::Skipped {
                    task_name: task.name.0.clone(),
                    timestamp: skipped_at,
                });
                task_records.push(skipped_task_record(task, skipped_at));
                continue;
            }

            observer.on_task_started(task.name.as_str());
            task_events.push(TaskEventRecord::Started {
                task_name: task.name.0.clone(),
                timestamp: self.pal.now(),
            });

            let (task_output, log_lines, execution_result) =
                execute_task(self.pal.clone(), recipe_directory.clone(), task.clone());
            append_task_output(&mut output, &task_output);

            match execution_result {
                Ok(result) => {
                    let task_failed = result.exit_code.unwrap_or(1) != 0;
                    if task_failed {
                        observer.on_task_failed(task.name.as_str());
                        failed = true;
                        let task_failure = TaskFailure {
                            task_name: task.name.0.clone(),
                            exit_code: result.exit_code.unwrap_or(-1),
                            elapsed_nanos: result
                                .finished_at
                                .as_nanos()
                                .saturating_sub(run_started_at.as_nanos()),
                            successful_task_count,
                            omitted_output_line_count: task_output_omitted_line_count(&log_lines),
                            output_tail_lines: task_output_tail_lines(&log_lines),
                        };
                        failure_message = Some(render_task_failure_message(&task_failure));
                        run_status = RunStatus::Failed(task_failure);
                    } else {
                        observer.on_task_completed(task.name.as_str());
                        successful_task_count += 1;
                    }

                    let status = if task_failed { "failed" } else { "completed" };
                    let result_name = if task_failed { "failed" } else { "success" };
                    task_events.push(TaskEventRecord::Finished {
                        task_name: task.name.0.clone(),
                        timestamp: result.finished_at,
                        status: SharedString::from(status),
                        result: SharedString::from(result_name),
                        exit_code: result.exit_code,
                    });
                    task_records.push(TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from(status),
                        result: SharedString::from(result_name),
                        started_at: Some(result.started_at),
                        finished_at: Some(result.finished_at),
                        exit_code: result.exit_code,
                        log_lines,
                    });
                }
                Err(error_message) => {
                    observer.on_task_failed(task.name.as_str());
                    failed = true;
                    let failed_at = self.pal.now();
                    failure_message = Some(render_task_execution_error_message(
                        task.name.as_str(),
                        failed_at
                            .as_nanos()
                            .saturating_sub(run_started_at.as_nanos()),
                        successful_task_count,
                        error_message.as_str(),
                    ));
                    run_status = RunStatus::Failed(TaskFailure {
                        task_name: task.name.0.clone(),
                        exit_code: -1,
                        elapsed_nanos: failed_at
                            .as_nanos()
                            .saturating_sub(run_started_at.as_nanos()),
                        successful_task_count,
                        omitted_output_line_count: task_output_omitted_line_count(&log_lines),
                        output_tail_lines: task_output_tail_lines(&log_lines),
                    });
                    task_events.push(TaskEventRecord::Finished {
                        task_name: task.name.0.clone(),
                        timestamp: failed_at,
                        status: SharedString::from("failed"),
                        result: SharedString::from("failed"),
                        exit_code: None,
                    });
                    task_records.push(TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from("failed"),
                        result: SharedString::from("failed"),
                        started_at: None,
                        finished_at: Some(failed_at),
                        exit_code: None,
                        log_lines,
                    });
                }
            }
        }

        Ok((
            output,
            task_records,
            task_events,
            run_status,
            failure_message,
        ))
    }

    fn execute_planned_run_concurrent(
        &self,
        recipe_directory: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
        run_started_at: Timestamp,
    ) -> NaoResult<TaskRunArtifacts> {
        let (dependents, mut remaining_prerequisites) = build_dependency_graph(&plan.tasks)?;
        let mut states = plan
            .tasks
            .iter()
            .map(|_| TaskRunState::Pending)
            .collect::<Vec<_>>();
        let mut ready_queue = VecDeque::new();
        for (task_index, prerequisite_count) in remaining_prerequisites.iter().enumerate() {
            if *prerequisite_count == 0 {
                states[task_index] = TaskRunState::Ready;
                ready_queue.push_back(task_index);
            }
        }

        let (sender, receiver) = mpsc::channel::<TaskExecutionMessage>();
        let mut join_handles = Vec::new();
        let mut running_count = 0usize;
        let mut output_by_task = vec![SharedString::empty(); plan.tasks.len()];
        let mut task_records = std::iter::repeat_with(|| None)
            .take(plan.tasks.len())
            .collect::<Vec<Option<TaskArtifactRecord>>>();
        let mut task_events = Vec::new();
        let mut successful_task_count = 0usize;
        let mut failure_message = None;
        let mut run_status = RunStatus::Completed;
        let mut stop_launching = false;

        while running_count > 0 || (!stop_launching && !ready_queue.is_empty()) {
            while !stop_launching
                && running_count < plan.max_parallel_tasks
                && !ready_queue.is_empty()
            {
                let task_index = ready_queue.pop_front().unwrap();
                let task = plan.tasks[task_index].clone();
                states[task_index] = TaskRunState::Running;
                observer.on_task_started(task.name.as_str());
                task_events.push(TaskEventRecord::Started {
                    task_name: task.name.0.clone(),
                    timestamp: self.pal.now(),
                });

                let worker_sender = sender.clone();
                let pal = self.pal.clone();
                let worker_recipe_directory = recipe_directory.clone();
                join_handles.push(thread::spawn(move || {
                    let (task_output, log_lines, execution_result) =
                        execute_task(pal, worker_recipe_directory, task);
                    worker_sender
                        .send((task_index, task_output, log_lines, execution_result))
                        .unwrap();
                }));
                running_count += 1;
            }

            if running_count == 0 {
                break;
            }

            let (task_index, task_output, log_lines, execution_result) = receiver.recv().unwrap();
            running_count = running_count.saturating_sub(1);
            output_by_task[task_index] = task_output;
            let task = &plan.tasks[task_index];

            match execution_result {
                Ok(result) => {
                    let task_failed = result.exit_code.unwrap_or(1) != 0;
                    if task_failed {
                        states[task_index] = TaskRunState::Failed;
                        observer.on_task_failed(task.name.as_str());
                        stop_launching = true;
                        if failure_message.is_none() {
                            let task_failure = TaskFailure {
                                task_name: task.name.0.clone(),
                                exit_code: result.exit_code.unwrap_or(-1),
                                elapsed_nanos: result
                                    .finished_at
                                    .as_nanos()
                                    .saturating_sub(run_started_at.as_nanos()),
                                successful_task_count,
                                omitted_output_line_count: task_output_omitted_line_count(
                                    &log_lines,
                                ),
                                output_tail_lines: task_output_tail_lines(&log_lines),
                            };
                            failure_message = Some(render_task_failure_message(&task_failure));
                            run_status = RunStatus::Failed(task_failure);
                        }
                    } else {
                        states[task_index] = TaskRunState::Completed;
                        observer.on_task_completed(task.name.as_str());
                        successful_task_count += 1;
                        if !stop_launching {
                            for dependent_index in &dependents[task_index] {
                                remaining_prerequisites[*dependent_index] =
                                    remaining_prerequisites[*dependent_index].saturating_sub(1);
                                if remaining_prerequisites[*dependent_index] == 0
                                    && states[*dependent_index] == TaskRunState::Pending
                                {
                                    states[*dependent_index] = TaskRunState::Ready;
                                    ready_queue.push_back(*dependent_index);
                                }
                            }
                        }
                    }

                    let status = if task_failed { "failed" } else { "completed" };
                    let result_name = if task_failed { "failed" } else { "success" };
                    task_events.push(TaskEventRecord::Finished {
                        task_name: task.name.0.clone(),
                        timestamp: result.finished_at,
                        status: SharedString::from(status),
                        result: SharedString::from(result_name),
                        exit_code: result.exit_code,
                    });
                    task_records[task_index] = Some(TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from(status),
                        result: SharedString::from(result_name),
                        started_at: Some(result.started_at),
                        finished_at: Some(result.finished_at),
                        exit_code: result.exit_code,
                        log_lines,
                    });
                }
                Err(error_message) => {
                    states[task_index] = TaskRunState::Failed;
                    observer.on_task_failed(task.name.as_str());
                    stop_launching = true;
                    let failed_at = self.pal.now();
                    if failure_message.is_none() {
                        failure_message = Some(render_task_execution_error_message(
                            task.name.as_str(),
                            failed_at
                                .as_nanos()
                                .saturating_sub(run_started_at.as_nanos()),
                            successful_task_count,
                            error_message.as_str(),
                        ));
                        run_status = RunStatus::Failed(TaskFailure {
                            task_name: task.name.0.clone(),
                            exit_code: -1,
                            elapsed_nanos: failed_at
                                .as_nanos()
                                .saturating_sub(run_started_at.as_nanos()),
                            successful_task_count,
                            omitted_output_line_count: task_output_omitted_line_count(&log_lines),
                            output_tail_lines: task_output_tail_lines(&log_lines),
                        });
                    }
                    task_events.push(TaskEventRecord::Finished {
                        task_name: task.name.0.clone(),
                        timestamp: failed_at,
                        status: SharedString::from("failed"),
                        result: SharedString::from("failed"),
                        exit_code: None,
                    });
                    task_records[task_index] = Some(TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from("failed"),
                        result: SharedString::from("failed"),
                        started_at: None,
                        finished_at: Some(failed_at),
                        exit_code: None,
                        log_lines,
                    });
                }
            }
        }

        drop(sender);
        for handle in join_handles {
            handle.join().unwrap();
        }

        if stop_launching {
            for (task_index, task) in plan.tasks.iter().enumerate() {
                if task_records[task_index].is_some() {
                    continue;
                }
                states[task_index] = TaskRunState::Skipped;
                observer.on_task_skipped(task.name.as_str());
                let skipped_at = self.pal.now();
                task_events.push(TaskEventRecord::Skipped {
                    task_name: task.name.0.clone(),
                    timestamp: skipped_at,
                });
                task_records[task_index] = Some(skipped_task_record(task, skipped_at));
            }
        }

        let mut output = SharedString::empty();
        for task_output in &output_by_task {
            append_task_output(&mut output, task_output);
        }

        Ok((
            output,
            task_records.into_iter().flatten().collect(),
            task_events,
            run_status,
            failure_message,
        ))
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

fn execute_task(
    pal: PalHandle,
    recipe_directory: FilePath,
    task: Task,
) -> (SharedString, TaskLogLines, TaskExecutionResult) {
    let mut framer = TaskOutputFramer::new();
    framer.push_task_heading(task.name.as_str());

    let execution_result = match build_process_command(&recipe_directory, &task) {
        Ok(command) => pal
            .run_process(&command, &mut framer)
            .map_err(|error| SharedString::from(error.to_test_string().as_str())),
        Err(error) => Err(SharedString::from(error.to_test_string().as_str())),
    };
    let (task_output, log_lines) = framer.into_parts();

    (task_output, log_lines, execution_result)
}

fn append_task_output(output: &mut SharedString, task_output: &SharedString) {
    if task_output.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push_str("\n");
    }
    output.push_str(task_output.as_str());
}

fn skipped_task_record(task: &Task, skipped_at: Timestamp) -> TaskArtifactRecord {
    TaskArtifactRecord {
        name: task.name.0.clone(),
        status: SharedString::from("skipped"),
        result: SharedString::from("skipped"),
        started_at: None,
        finished_at: Some(skipped_at),
        exit_code: None,
        log_lines: Vec::new(),
    }
}

fn build_dependency_graph(tasks: &[Task]) -> NaoResult<(Vec<Vec<usize>>, Vec<usize>)> {
    let task_index = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut dependents = vec![Vec::new(); tasks.len()];
    let mut remaining_prerequisites = vec![0usize; tasks.len()];

    for (task_position, task) in tasks.iter().enumerate() {
        remaining_prerequisites[task_position] = task.dependencies.len();
        for dependency in &task.dependencies {
            let dependency_index = task_index
                .get(dependency.as_str())
                .copied()
                .ok_or_else(|| err!("task `{}` not found", dependency.as_str()))?;
            dependents[dependency_index].push(task_position);
        }
    }

    Ok((dependents, remaining_prerequisites))
}

struct NoopRunObserver;

impl RunObserver for NoopRunObserver {}

fn split_task_selectors(task_names: &[String]) -> Vec<String> {
    task_names
        .iter()
        .flat_map(|task_name| task_name.split(','))
        .map(str::trim)
        .filter(|task_name| !task_name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn expand_task_selectors(tasks: &[Task], selectors: &[String]) -> NaoResult<Vec<String>> {
    let mut expanded = Vec::new();

    for selector in selectors {
        if selector.contains('_') {
            expanded.extend(expand_wildcard_selector(tasks, selector)?);
        } else {
            expanded.push(selector.clone());
        }
    }

    Ok(expanded)
}

/* 📖 # Why do task specifiers use `_` as the wildcard instead of `*`?
Using `*` would force callers to quote task specifiers in most shells because the shell expands
asterisks before `nao` sees the argument. `_` keeps wildcard task selection available from the
command line without extra quoting or platform-specific escaping rules.
*/
fn expand_wildcard_selector(tasks: &[Task], selector: &str) -> NaoResult<Vec<String>> {
    let pattern_parts = selector.split('_').collect::<Vec<_>>();
    let matches = tasks
        .iter()
        .filter(|task| task_name_matches_selector(task.name.as_str(), &pattern_parts))
        .map(|task| task.name.as_str().to_owned())
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Err(err!("task specifier `{selector}` did not match any tasks"));
    }

    Ok(matches)
}

fn task_name_matches_selector(task_name: &str, pattern_parts: &[&str]) -> bool {
    let mut remaining = task_name;

    for (index, part) in pattern_parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if index == 0 {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
            continue;
        }

        match remaining.find(part) {
            Some(position) => {
                remaining = &remaining[position + part.len()..];
            }
            None => return false,
        }
    }

    selector_has_trailing_wildcard(pattern_parts) || remaining.is_empty()
}

fn selector_has_trailing_wildcard(pattern_parts: &[&str]) -> bool {
    pattern_parts.last().is_some_and(|part| part.is_empty())
}

fn render_task_failure_message(task_failure: &TaskFailure) -> String {
    format!(
        "task `{}` failed with exit code {} after {} ({} completed successfully)",
        task_failure.task_name.as_str(),
        task_failure.exit_code,
        pretty_duration(task_failure.elapsed_nanos),
        render_completed_task_count(task_failure.successful_task_count),
    )
}

fn task_output_omitted_line_count(
    log_lines: &[(nao_base::timestamp::Timestamp, ProcessOutputStream, String)],
) -> usize {
    log_lines.len().saturating_sub(100)
}

fn task_output_tail_lines(
    log_lines: &[(nao_base::timestamp::Timestamp, ProcessOutputStream, String)],
) -> Vec<SharedString> {
    log_lines
        .iter()
        .skip(log_lines.len().saturating_sub(100))
        .map(|(_, _, line)| SharedString::from(line.as_str()))
        .collect()
}

fn render_task_execution_error_message(
    task_name: &str,
    elapsed_nanos: u128,
    successful_task_count: usize,
    error: &str,
) -> String {
    format!(
        "task `{task_name}` failed after {} ({} completed successfully): {error}",
        pretty_duration(elapsed_nanos),
        render_completed_task_count(successful_task_count),
    )
}

fn render_completed_task_count(successful_task_count: usize) -> String {
    format!(
        "{successful_task_count} {}",
        if successful_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
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

fn build_process_command(recipe_directory: &FilePath, task: &Task) -> NaoResult<ProcessCommand> {
    let environment = task
        .environment
        .iter()
        .map(|variable| ProcessEnvironmentVariable {
            name: variable.name.clone(),
            value: variable.value.clone(),
        })
        .collect::<Vec<_>>();

    match &task.run {
        RunSpec::Shell(command) => {
            #[cfg(windows)]
            let (executable, arguments) = (
                SharedString::from("cmd"),
                vec![SharedString::from("/C"), command.clone()],
            );

            #[cfg(not(windows))]
            let (executable, arguments) = (
                SharedString::from("sh"),
                vec![SharedString::from("-c"), command.clone()],
            );

            Ok(ProcessCommand {
                executable,
                arguments,
                working_directory: Some(recipe_directory.clone()),
                environment,
            })
        }
        RunSpec::Script(script) => Ok(ProcessCommand {
            executable: SharedString::from(script.as_str()),
            arguments: Vec::new(),
            working_directory: Some(recipe_directory.clone()),
            environment,
        }),
        RunSpec::Container(container) => Err(err!(
            "container execution is not implemented yet for task `{}` with image `{}`",
            task.name.as_str(),
            container.image.as_str()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::RunEngine;
    use crate::run_execution_result::RunStatus;
    use crate::run_execution_result::TaskFailure;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_base::timestamp::Timestamp;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use nao_pal::process_command::ProcessCommand;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_output_event::ProcessOutputEvent;
    use nao_pal::process_output_stream::ProcessOutputStream;
    use nao_pal::process_result::ProcessResult;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
    use nao_recipe::LiveDisplay;
    use std::time::{Duration, SystemTime};

    fn test_engine() -> RunEngine {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build the workspace" {
                run shell="cargo build --workspace --all-targets --all-features"
              }

              task "test" description="Run the test suite" {
                depends-on "build"
                run shell="cargo nextest run --workspace --all-targets --all-features"
              }
            }
            "#,
        );
        RunEngine::new(PalHandle::new(pal))
    }

    fn set_script_process(pal: &PalMock, script: &str, chunks: &[&[u8]], exit_code: i32) {
        set_script_process_with_delay(pal, script, chunks, exit_code, Duration::ZERO);
    }

    fn set_script_process_with_delay(
        pal: &PalMock,
        script: &str,
        chunks: &[&[u8]],
        exit_code: i32,
        delay: Duration,
    ) {
        let mut events = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            events.push(ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new((index + 1) as u128),
                stream: ProcessOutputStream::Stdout,
                bytes: chunk.to_vec(),
            }));
        }
        events.push(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
            timestamp: Timestamp::new((chunks.len() + 1) as u128),
            stream: ProcessOutputStream::Stdout,
        }));
        events.push(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
            timestamp: Timestamp::new((chunks.len() + 2) as u128),
            stream: ProcessOutputStream::Stderr,
        }));
        events.push(ProcessEvent::Exited(ProcessExitedEvent {
            timestamp: Timestamp::new((chunks.len() + 3) as u128),
            exit_code: Some(exit_code),
        }));

        pal.set_process_execution_with_delay(
            ProcessCommand {
                executable: script.into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            events,
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new((chunks.len() + 3) as u128),
                exit_code: Some(exit_code),
            },
            delay,
        );
    }

    #[test]
    fn lists_recipe_tasks() {
        let tasks = test_engine()
            .list_tasks(&FilePath::from("nao.kdl"))
            .unwrap();
        let task_names = tasks
            .iter()
            .map(|task| task.name.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        expect![
            r#"build
test"#
        ]
        .assert_eq(&task_names);
    }

    #[test]
    fn plans_requested_tasks() {
        let plan = test_engine()
            .plan_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=test
planned=build,test"#
        ]
        .assert_eq(&rendered);
        assert_eq!(plan.live_display, LiveDisplay::LinePerTask);
        assert_eq!(plan.max_parallel_tasks, 1);
    }

    #[test]
    fn plans_requested_live_display_mode() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              config live-display="single-line"

              task "test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        assert_eq!(plan.live_display, LiveDisplay::SingleLine);
    }

    #[test]
    fn plans_requested_max_parallel_tasks() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              config max-parallel-tasks=3

              task "test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        assert_eq!(plan.max_parallel_tasks, 3);
    }

    #[test]
    fn defaults_planned_parallel_tasks_from_pal() {
        let pal = PalMock::new();
        pal.set_default_parallelism(6);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        assert_eq!(plan.max_parallel_tasks, 6);
    }

    #[test]
    fn plans_comma_separated_requested_tasks() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" {
                run script="./scripts/build.sh"
              }

              task "lint" {
                run script="./scripts/lint.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["lint,test".to_owned()])
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=lint,test
planned=lint,build,test"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn plans_mixed_comma_separated_and_repeated_requested_tasks() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" {
                run script="./scripts/build.sh"
              }

              task "lint" {
                run script="./scripts/lint.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(
                &FilePath::from("nao.kdl"),
                &["lint,test".to_owned(), "build".to_owned()],
            )
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=lint,test,build
planned=lint,build,test"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn plans_wildcard_requested_tasks() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "slow" {
                run script="./scripts/slow.sh"
              }

              task "slow1" {
                run script="./scripts/slow1.sh"
              }

              task "slowpoke" {
                run script="./scripts/slowpoke.sh"
              }

              task "fast" {
                run script="./scripts/fast.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["slow_".to_owned()])
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=slow,slow1,slowpoke
planned=slow,slow1,slowpoke"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn plans_mixed_wildcard_and_comma_separated_requested_tasks() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "slow" {
                run script="./scripts/slow.sh"
              }

              task "slow1" {
                run script="./scripts/slow1.sh"
              }

              task "slowpoke" {
                run script="./scripts/slowpoke.sh"
              }

              task "fast" {
                run script="./scripts/fast.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let plan = engine
            .plan_run(&FilePath::from("nao.kdl"), &["slow_,fast".to_owned()])
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=slow,slow1,slowpoke,fast
planned=slow,slow1,slowpoke,fast"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn errors_when_wildcard_requested_tasks_match_nothing() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "fast" {
                run script="./scripts/fast.sh"
              }
            }
            "#,
        );
        let engine = RunEngine::new(PalHandle::new(pal));
        let error = engine
            .plan_run(&FilePath::from("nao.kdl"), &["slow_".to_owned()])
            .unwrap_err();

        expect![[r#"
            × error task specifier `slow_` did not match any tasks
              at crates/engine/src/run_engine.rs:658:20
        "#]]
        .assert_eq(&error.to_test_string());
    }

    #[test]
    fn executes_tasks_in_dependency_order() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" {
                run script="./scripts/build.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        set_script_process(&pal, "./scripts/build.sh", &[b"building\n"], 0);
        set_script_process(&pal, "./scripts/test.sh", &[b"testing"], 0);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        let output = engine
            .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        expect![
            r#"Running task `build`
[1ns] stdout: building
[4ns] process exited with code 0

Running task `test`
[2ns] stdout: testing
[4ns] process exited with code 0
"#
        ]
        .assert_eq(output.output.as_str());
        assert_eq!(output.goal_tasks, vec![SharedString::from("test")]);
        assert_eq!(output.total_task_count, 2);
        assert_eq!(output.duration_nanos, 0);
        assert_eq!(output.status, RunStatus::Completed);
        pal.verify_effects(expect![
            r#"READ FILE: nao.kdl
CREATE DIRECTORY: .nao/runs
CREATE DIRECTORY: .nao/runs/1970-01-01T00-00-00Z-test
WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-plan.json -> {
  "requested_tasks": [
    "test"
  ],
  "tasks": [
    {
      "artifacts": [],
      "dependencies": [],
      "description": null,
      "environment": [],
      "name": "build",
      "run": {
        "kind": "script",
        "path": "./scripts/build.sh"
      }
    },
    {
      "artifacts": [],
      "dependencies": [
        "build"
      ],
      "description": null,
      "environment": [],
      "name": "test",
      "run": {
        "kind": "script",
        "path": "./scripts/test.sh"
      }
    }
  ]
}
RUN PROCESS: ./scripts/build.sh 
RUN PROCESS: ./scripts/test.sh 
WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/build.log -> [1970-01-01T00:00:00Z] stdout: building

WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/test.log -> [1970-01-01T00:00:00Z] stdout: testing

WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"requested_tasks":["test"],"timestamp":"1970-01-01T00:00:00Z","type":"run_started"}
{"task":"build","timestamp":"1970-01-01T00:00:00Z","type":"task_started"}
{"exit_code":0,"result":"success","status":"completed","task":"build","timestamp":"1970-01-01T00:00:00Z","type":"task_finished"}
{"task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_started"}
{"exit_code":0,"result":"success","status":"completed","task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_finished"}
{"result":"completed","timestamp":"1970-01-01T00:00:00Z","type":"run_finished"}

WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-summary.json -> {
  "failure_message": null,
  "result": "completed",
  "run": {
    "duration_nanos": "0",
    "finished_at": "1970-01-01T00:00:00Z",
    "requested_tasks": [
      "test"
    ],
    "started_at": "1970-01-01T00:00:00Z"
  },
  "tasks": [
    {
      "duration_nanos": "4",
      "exit_code": 0,
      "finished_at": "1970-01-01T00:00:00Z",
      "log_file": "build.log",
      "name": "build",
      "result": "success",
      "started_at": "1970-01-01T00:00:00Z",
      "status": "completed"
    },
    {
      "duration_nanos": "4",
      "exit_code": 0,
      "finished_at": "1970-01-01T00:00:00Z",
      "log_file": "test.log",
      "name": "test",
      "result": "success",
      "started_at": "1970-01-01T00:00:00Z",
      "status": "completed"
    }
  ]
}
"#
        ]);
    }

    #[test]
    fn writes_failed_run_summary_and_skipped_tasks() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" {
                run script="./scripts/build.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }

              task "package" {
                depends-on "test"
                run script="./scripts/package.sh"
              }
            }
            "#,
        );
        set_script_process(&pal, "./scripts/build.sh", &[b"building\n"], 0);
        set_script_process(&pal, "./scripts/test.sh", &[b"boom\n"], 1);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        let result = engine
            .execute_run(&FilePath::from("nao.kdl"), &["package".to_owned()])
            .unwrap();

        assert_eq!(
            result.status,
            RunStatus::Failed(TaskFailure {
                task_name: SharedString::from("test"),
                exit_code: 1,
                elapsed_nanos: 4,
                successful_task_count: 1,
                omitted_output_line_count: 0,
                output_tail_lines: vec![SharedString::from("boom")],
            })
        );

        let summary = pal
            .read_file_string(".nao/runs/1970-01-01T00-00-10Z-package/nao-summary.json")
            .unwrap();
        assert!(summary.contains("\"result\": \"failed\""));
        assert!(summary.contains("\"name\": \"package\""));
        assert!(summary.contains("\"status\": \"skipped\""));
        assert!(
            summary.contains(
                "\"failure_message\": \"task `test` failed with exit code 1 after 4ns (1 task completed successfully)\""
            )
        );
    }

    #[test]
    fn executes_independent_tasks_concurrently() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              config max-parallel-tasks=2

              task "lint" {
                run script="./scripts/lint.sh"
              }

              task "fmt" {
                run script="./scripts/fmt.sh"
              }
            }
            "#,
        );
        set_script_process_with_delay(
            &pal,
            "./scripts/lint.sh",
            &[b"linting\n"],
            0,
            Duration::from_millis(30),
        );
        set_script_process(&pal, "./scripts/fmt.sh", &[b"formatting\n"], 0);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        let result = engine
            .execute_run(&FilePath::from("nao.kdl"), &["lint,fmt".to_owned()])
            .unwrap();

        assert_eq!(result.status, RunStatus::Completed);
        let events = pal
            .read_file_string(".nao/runs/1970-01-01T00-00-00Z-lint+fmt/nao-events.jsonl")
            .unwrap();
        let lint_started = events
            .lines()
            .position(|line| {
                line.contains("\"task\":\"lint\"") && line.contains("\"type\":\"task_started\"")
            })
            .unwrap();
        let fmt_started = events
            .lines()
            .position(|line| {
                line.contains("\"task\":\"fmt\"") && line.contains("\"type\":\"task_started\"")
            })
            .unwrap();
        let fmt_finished = events
            .lines()
            .position(|line| {
                line.contains("\"task\":\"fmt\"") && line.contains("\"type\":\"task_finished\"")
            })
            .unwrap();

        assert!(lint_started < fmt_finished);
        assert!(fmt_started < fmt_finished);
    }

    #[test]
    fn starts_dependents_only_after_prerequisites_finish() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              config max-parallel-tasks=2

              task "build" {
                run script="./scripts/build.sh"
              }

              task "lint" {
                run script="./scripts/lint.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        set_script_process_with_delay(
            &pal,
            "./scripts/build.sh",
            &[b"building\n"],
            0,
            Duration::from_millis(30),
        );
        set_script_process(&pal, "./scripts/lint.sh", &[b"linting\n"], 0);
        set_script_process(&pal, "./scripts/test.sh", &[b"testing\n"], 0);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        engine
            .execute_run(&FilePath::from("nao.kdl"), &["test,lint".to_owned()])
            .unwrap();

        let events = pal
            .read_file_string(".nao/runs/1970-01-01T00-00-00Z-test+lint/nao-events.jsonl")
            .unwrap();
        let build_finished = events
            .lines()
            .position(|line| {
                line.contains("\"task\":\"build\"") && line.contains("\"type\":\"task_finished\"")
            })
            .unwrap();
        let test_started = events
            .lines()
            .position(|line| {
                line.contains("\"task\":\"test\"") && line.contains("\"type\":\"task_started\"")
            })
            .unwrap();

        assert!(build_finished < test_started);
    }

    #[test]
    fn stops_launching_new_tasks_after_concurrent_failure() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              config max-parallel-tasks=2

              task "slow" {
                run script="./scripts/slow.sh"
              }

              task "fail" {
                run script="./scripts/fail.sh"
              }

              task "after-slow" {
                depends-on "slow"
                run script="./scripts/after-slow.sh"
              }
            }
            "#,
        );
        set_script_process_with_delay(
            &pal,
            "./scripts/slow.sh",
            &[b"slow\n"],
            0,
            Duration::from_millis(30),
        );
        set_script_process(&pal, "./scripts/fail.sh", &[b"boom\n"], 1);
        set_script_process(&pal, "./scripts/after-slow.sh", &[b"after\n"], 0);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        let result = engine
            .execute_run(&FilePath::from("nao.kdl"), &["after-slow,fail".to_owned()])
            .unwrap();

        assert!(matches!(result.status, RunStatus::Failed(_)));
        let summary = pal
            .read_file_string(".nao/runs/1970-01-01T00-00-00Z-after-slow+fail/nao-summary.json")
            .unwrap();
        let events = pal
            .read_file_string(".nao/runs/1970-01-01T00-00-00Z-after-slow+fail/nao-events.jsonl")
            .unwrap();

        assert!(summary.contains("\"name\": \"slow\""));
        assert!(summary.contains("\"status\": \"completed\""));
        assert!(summary.contains("\"name\": \"after-slow\""));
        assert!(summary.contains("\"status\": \"skipped\""));
        assert!(
            events
                .lines()
                .any(|line| line.contains("\"task\":\"after-slow\"")
                    && line.contains("\"type\":\"task_skipped\""))
        );
        assert!(
            !events
                .lines()
                .any(|line| line.contains("\"task\":\"after-slow\"")
                    && line.contains("\"type\":\"task_started\""))
        );
    }
}
