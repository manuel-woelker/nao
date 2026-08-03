use super::RunEngine;
use super::TASK_OUTCOME_PREFIX;
use super::TaskExecutionMessage;
use super::TaskExecutionResult;
use super::TaskLogLines;
use super::TaskRunArtifacts;
use crate::live_task_artifact_sink::LiveTaskArtifactSink;
use crate::planned_run::PlannedRun;
use crate::run_artifact_writer::RunArtifactWriter;
use crate::run_artifact_writer::TaskArtifactRecord;
use crate::run_execution_result::RunStatus;
use crate::run_execution_result::TaskFailure;
use crate::run_observer::RunObserver;
use crate::task_event_record::TaskEventRecord;
use crate::task_run_state::TaskRunState;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::result::{OptionExt, ResultExt};
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::pal::PalHandle;
use nao_pal::process_output_stream::ProcessOutputStream;
use nao_recipe::{FailureMode, Task, TaskName};
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc;
use std::thread;

impl RunEngine {
    pub(super) fn execute_planned_run_with_scheduler(
        &self,
        recipe_path: &FilePath,
        plan: &PlannedRun,
        observer: &mut dyn RunObserver,
        run_started_at: Timestamp,
        writer: &RunArtifactWriter,
    ) -> NaoResult<TaskRunArtifacts> {
        let max_parallel_tasks = plan.max_parallel_tasks.max(1);
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
            while !stop_launching && running_count < max_parallel_tasks && !ready_queue.is_empty() {
                let task_index = ready_queue.pop_front().with_context(|| {
                    "ready queue was empty while scheduling runnable tasks".to_owned()
                })?;
                if states[task_index] != TaskRunState::Ready {
                    continue;
                }
                let task = plan.tasks[task_index].clone();
                states[task_index] = TaskRunState::Running;
                observer.on_task_started(task.name.as_str());
                task_events.push(TaskEventRecord::Started {
                    task_name: task.name.0.clone(),
                    timestamp: self.pal.now(),
                });
                writer.append_task_started(task.name.as_str(), self.pal.now())?;

                let worker_sender = sender.clone();
                let pal = self.pal.clone();
                let worker_recipe_path = recipe_path.clone();
                let worker_writer = writer.clone();
                let task_name = task.name.0.clone();
                join_handles.push(thread::spawn(move || {
                    let (task_output, log_lines, execution_result, worker_sender) = execute_task(
                        pal,
                        worker_recipe_path,
                        task,
                        worker_writer,
                        task_index,
                        worker_sender,
                    );
                    worker_sender
                        .send(TaskExecutionMessage::Finished {
                            task_index,
                            output: task_output,
                            log_lines,
                            result: execution_result,
                        })
                        .with_context(|| {
                            format!(
                                "failed to send execution result for task `{}`",
                                task_name.as_str()
                            )
                        })
                }));
                running_count += 1;
            }

            if running_count == 0 {
                break;
            }

            let (task_index, task_output, log_lines, execution_result) = loop {
                match receiver
                    .recv()
                    .context("failed to receive task execution update from worker thread")?
                {
                    TaskExecutionMessage::Status {
                        task_index,
                        message,
                    } => {
                        observer
                            .on_task_status(plan.tasks[task_index].name.as_str(), message.as_str());
                    }
                    TaskExecutionMessage::OutputLine {
                        task_index,
                        stream,
                        line,
                    } => {
                        observer.on_task_output_line(
                            plan.tasks[task_index].name.as_str(),
                            stream,
                            line.as_str(),
                        );
                    }
                    TaskExecutionMessage::Finished {
                        task_index,
                        output,
                        log_lines,
                        result,
                    } => {
                        break (task_index, output, log_lines, result);
                    }
                }
            };
            running_count = running_count.saturating_sub(1);
            output_by_task[task_index] = task_output;
            let task = &plan.tasks[task_index];

            match execution_result {
                Ok(result) => {
                    let outcome_message = extract_task_outcome_message(&log_lines);
                    let task_failed = result.exit_code.unwrap_or(1) != 0;
                    if task_failed {
                        states[task_index] = TaskRunState::Failed;
                        observer.on_task_failed(
                            task.name.as_str(),
                            result
                                .finished_at
                                .as_nanos()
                                .saturating_sub(result.started_at.as_nanos()),
                            outcome_message.as_deref(),
                        );
                        if plan.failure_mode == FailureMode::FailEarly {
                            stop_launching = true;
                        } else {
                            SkipContext {
                                plan,
                                dependents: &dependents,
                                states: &mut states,
                                task_records: &mut task_records,
                                task_events: &mut task_events,
                                observer,
                                pal: &*self.pal,
                                writer,
                            }
                            .skip_dependents_after_failure(task_index)?;
                        }
                        if failure_message.is_none() {
                            let task_failure = TaskFailure {
                                task_name: task.name.0.clone(),
                                exit_code: result.exit_code.unwrap_or(-1),
                                elapsed_nanos: task_elapsed_nanos(
                                    result.started_at,
                                    result.finished_at,
                                ),
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
                        observer.on_task_completed(
                            task.name.as_str(),
                            result
                                .finished_at
                                .as_nanos()
                                .saturating_sub(result.started_at.as_nanos()),
                            outcome_message.as_deref(),
                        );
                        successful_task_count += 1;
                        for dependent_index in &dependents[task_index] {
                            remaining_prerequisites[*dependent_index] =
                                remaining_prerequisites[*dependent_index].saturating_sub(1);
                            if !stop_launching
                                && remaining_prerequisites[*dependent_index] == 0
                                && states[*dependent_index] == TaskRunState::Pending
                            {
                                states[*dependent_index] = TaskRunState::Ready;
                                ready_queue.push_back(*dependent_index);
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
                        outcome_message: outcome_message.as_deref().map(SharedString::from),
                    });
                    let task_record = TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from(status),
                        result: SharedString::from(result_name),
                        started_at: Some(result.started_at),
                        finished_at: Some(result.finished_at),
                        exit_code: result.exit_code,
                        outcome_message: outcome_message.map(SharedString::from),
                        log_lines,
                    };
                    writer.append_task_finished(&task_record)?;
                    task_records[task_index] = Some(task_record);
                }
                Err(error_message) => {
                    states[task_index] = TaskRunState::Failed;
                    let outcome_message = extract_task_outcome_message(&log_lines);
                    observer.on_task_failed(task.name.as_str(), 0, outcome_message.as_deref());
                    if plan.failure_mode == FailureMode::FailEarly {
                        stop_launching = true;
                    } else {
                        SkipContext {
                            plan,
                            dependents: &dependents,
                            states: &mut states,
                            task_records: &mut task_records,
                            task_events: &mut task_events,
                            observer,
                            pal: &*self.pal,
                            writer,
                        }
                        .skip_dependents_after_failure(task_index)?;
                    }
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
                        outcome_message: outcome_message.as_deref().map(SharedString::from),
                    });
                    let task_record = TaskArtifactRecord {
                        name: task.name.0.clone(),
                        status: SharedString::from("failed"),
                        result: SharedString::from("failed"),
                        started_at: None,
                        finished_at: Some(failed_at),
                        exit_code: None,
                        outcome_message: outcome_message.map(SharedString::from),
                        log_lines,
                    };
                    writer.append_task_finished(&task_record)?;
                    task_records[task_index] = Some(task_record);
                }
            }
        }

        drop(sender);
        for handle in join_handles {
            handle
                .join()
                .map_err(|_| err!("task worker thread panicked"))
                .context("failed to join task worker thread")??;
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
                writer.append_task_skipped(task.name.as_str(), skipped_at)?;
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
}

pub(super) struct NoopRunObserver;

impl RunObserver for NoopRunObserver {}

fn execute_task(
    pal: PalHandle,
    recipe_path: FilePath,
    task: Task,
    writer: RunArtifactWriter,
    task_index: usize,
    sender: mpsc::Sender<TaskExecutionMessage>,
) -> (
    SharedString,
    TaskLogLines,
    TaskExecutionResult,
    mpsc::Sender<TaskExecutionMessage>,
) {
    let mut framer = LiveTaskArtifactSink::new(
        writer,
        task.name.0.clone(),
        task_index,
        sender,
        task.direct_output,
    );

    let execution_result =
        match crate::run_engine::process_command::build_process_command(&recipe_path, &task) {
            Ok(command) => pal
                .run_process(&command, &mut framer)
                .map_err(|error| SharedString::from(error.to_test_string().as_str())),
            Err(error) => Err(SharedString::from(error.to_test_string().as_str())),
        };
    let (task_output, log_lines, sender) = framer.into_parts();

    (task_output, log_lines, execution_result, sender)
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

fn task_elapsed_nanos(started_at: Timestamp, finished_at: Timestamp) -> u128 {
    finished_at.as_nanos().saturating_sub(started_at.as_nanos())
}

fn skipped_task_record(task: &Task, skipped_at: Timestamp) -> TaskArtifactRecord {
    TaskArtifactRecord {
        name: task.name.0.clone(),
        status: SharedString::from("skipped"),
        result: SharedString::from("skipped"),
        started_at: None,
        finished_at: Some(skipped_at),
        exit_code: None,
        outcome_message: None,
        log_lines: Vec::new(),
    }
}

struct SkipContext<'a> {
    plan: &'a PlannedRun,
    dependents: &'a [Vec<usize>],
    states: &'a mut [TaskRunState],
    task_records: &'a mut [Option<TaskArtifactRecord>],
    task_events: &'a mut Vec<TaskEventRecord>,
    observer: &'a mut dyn RunObserver,
    pal: &'a dyn nao_pal::pal::Pal,
    writer: &'a RunArtifactWriter,
}

impl SkipContext<'_> {
    fn skip_dependents_after_failure(&mut self, failed_task_index: usize) -> NaoResult<()> {
        let mut queue = VecDeque::from(self.dependents[failed_task_index].clone());

        while let Some(task_index) = queue.pop_front() {
            if self.task_records[task_index].is_some() {
                continue;
            }
            match self.states[task_index] {
                TaskRunState::Pending | TaskRunState::Ready => {
                    self.states[task_index] = TaskRunState::Skipped;
                    let task = &self.plan.tasks[task_index];
                    self.observer.on_task_skipped(task.name.as_str());
                    let skipped_at = self.pal.now();
                    self.task_events.push(TaskEventRecord::Skipped {
                        task_name: task.name.0.clone(),
                        timestamp: skipped_at,
                    });
                    self.writer
                        .append_task_skipped(task.name.as_str(), skipped_at)?;
                    self.task_records[task_index] = Some(skipped_task_record(task, skipped_at));
                    for dependent_index in &self.dependents[task_index] {
                        queue.push_back(*dependent_index);
                    }
                }
                TaskRunState::Skipped => {}
                TaskRunState::Running | TaskRunState::Completed | TaskRunState::Failed => {}
            }
        }

        Ok(())
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

fn render_task_failure_message(task_failure: &TaskFailure) -> String {
    format!(
        "task `{}` failed with exit code {} after {} ({} completed successfully)",
        task_failure.task_name.as_str(),
        task_failure.exit_code,
        pretty_duration(task_failure.elapsed_nanos),
        render_completed_task_count(task_failure.successful_task_count),
    )
}

fn task_output_omitted_line_count(log_lines: &[(Timestamp, ProcessOutputStream, String)]) -> usize {
    log_lines.len().saturating_sub(200)
}

fn task_output_tail_lines(
    log_lines: &[(Timestamp, ProcessOutputStream, String)],
) -> Vec<SharedString> {
    log_lines
        .iter()
        .skip(log_lines.len().saturating_sub(200))
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

pub(super) fn pretty_duration(duration_nanos: u128) -> String {
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

pub(super) fn extract_task_outcome_message(
    log_lines: &[(Timestamp, ProcessOutputStream, String)],
) -> Option<String> {
    log_lines.iter().fold(None, |latest_outcome, (_, _, line)| {
        match line
            .strip_prefix(TASK_OUTCOME_PREFIX)
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            Some(message) => Some(message.to_owned()),
            None => latest_outcome,
        }
    })
}

pub(super) fn goal_outcome_message(
    goal_tasks: &[TaskName],
    task_records: &[TaskArtifactRecord],
) -> Option<SharedString> {
    let [goal_task] = goal_tasks else {
        return None;
    };

    task_records
        .iter()
        .find(|task_record| {
            task_record.name.as_str() == goal_task.as_str()
                && task_record.status.as_str() == "completed"
        })
        .and_then(|task_record| task_record.outcome_message.clone())
}
