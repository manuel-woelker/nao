use crate::planned_run::PlannedRun;
use crate::task_event_record::TaskEventRecord;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::pal::PalHandle;
use nao_pal::process_output_stream::ProcessOutputStream;
use nao_recipe::RunSpec;
use nao_recipe::Task;
use serde_json::json;
use std::time::{Duration, SystemTime};
use time::OffsetDateTime;

/// Writes `.nao/runs` artifacts for one executed run.
pub struct RunArtifactWriter {
    pal: PalHandle,
    run_root_directory: FilePath,
    run_directory: FilePath,
    run_started_at: Timestamp,
    run_started_system_time: SystemTime,
}

/// Captures the persisted outcome of one planned task.
pub struct TaskArtifactRecord {
    /// Task name.
    pub name: SharedString,
    /// Final task status.
    pub status: SharedString,
    /// Final task result string.
    pub result: SharedString,
    /// Task start timestamp when the task started.
    pub started_at: Option<Timestamp>,
    /// Task finish timestamp when the task finished or was skipped.
    pub finished_at: Option<Timestamp>,
    /// Process exit code when available.
    pub exit_code: Option<i32>,
    /// Timestamped log lines emitted by the task.
    pub log_lines: Vec<(Timestamp, ProcessOutputStream, String)>,
}

impl RunArtifactWriter {
    /// Creates a run artifact writer for one planned invocation.
    pub fn new(
        pal: PalHandle,
        recipe_directory: &FilePath,
        requested_task_names: &[String],
        run_started_at: Timestamp,
        run_started_system_time: SystemTime,
    ) -> Self {
        let run_root_directory = recipe_directory.join(".nao").join("runs").normalize();
        let run_directory_name = format!(
            "{}-{}",
            format_file_safe_iso8601(run_started_system_time),
            sanitize_file_component(&requested_task_names.join("+"))
        );
        let run_directory = run_root_directory.join(run_directory_name).normalize();

        Self {
            pal,
            run_root_directory,
            run_directory,
            run_started_at,
            run_started_system_time,
        }
    }

    /// Returns the run directory for this execution.
    pub fn run_directory(&self) -> FilePath {
        self.run_directory.clone()
    }

    /// Creates the run directory and writes the planned run description.
    pub fn write_plan(&self, planned_run: &PlannedRun) -> NaoResult<()> {
        self.pal.create_directory_all(&self.run_root_directory)?;
        self.pal.create_directory_all(&self.run_directory)?;
        self.pal.write_file(
            &self.run_directory.join("nao-plan.json"),
            serde_json::to_vec_pretty(&json!({
                "requested_tasks": planned_run
                    .requested_tasks
                    .iter()
                    .map(|task| task.as_str())
                    .collect::<Vec<_>>(),
                "tasks": planned_run
                    .tasks
                    .iter()
                    .map(task_plan_json)
                    .collect::<Vec<_>>(),
            }))?
            .as_slice(),
        )?;
        Ok(())
    }

    /// Writes task logs, event lines, and the final run summary.
    pub fn write_completion(
        &self,
        planned_run: &PlannedRun,
        task_records: &[TaskArtifactRecord],
        task_events: &[TaskEventRecord],
        run_finished_at: Timestamp,
        overall_result: &str,
        failure_message: Option<&str>,
    ) -> NaoResult<()> {
        for task_record in task_records {
            self.pal.write_file(
                &self.run_directory.join(format!(
                    "{}.log",
                    sanitize_file_component(task_record.name.as_str())
                )),
                render_task_log(
                    self.run_started_system_time,
                    self.run_started_at,
                    &task_record.log_lines,
                )
                .as_bytes(),
            )?;
        }

        self.pal.write_file(
            &self.run_directory.join("nao-events.jsonl"),
            render_events_jsonl(
                self.run_started_system_time,
                self.run_started_at,
                planned_run,
                task_events,
                run_finished_at,
                overall_result,
            )
            .as_bytes(),
        )?;

        self.pal.write_file(
            &self.run_directory.join("nao-summary.json"),
            serde_json::to_vec_pretty(&json!({
                "result": overall_result,
                "failure_message": failure_message,
                "run": {
                    "requested_tasks": planned_run
                        .requested_tasks
                        .iter()
                        .map(|task| task.as_str())
                        .collect::<Vec<_>>(),
                    "started_at": format_iso8601(self.run_started_system_time),
                    "finished_at": format_iso8601(absolute_system_time(
                        self.run_started_system_time,
                        self.run_started_at,
                        run_finished_at,
                    )),
                    "duration_nanos": run_finished_at
                        .as_nanos()
                        .saturating_sub(self.run_started_at.as_nanos())
                        .to_string(),
                },
                "tasks": task_records
                    .iter()
                    .map(|task_record| {
                        json!({
                            "name": task_record.name.as_str(),
                            "status": task_record.status.as_str(),
                            "result": task_record.result.as_str(),
                            "exit_code": task_record.exit_code,
                            "started_at": task_record.started_at.map(|timestamp| format_iso8601(absolute_system_time(
                                self.run_started_system_time,
                                self.run_started_at,
                                timestamp,
                            ))),
                            "finished_at": task_record.finished_at.map(|timestamp| format_iso8601(absolute_system_time(
                                self.run_started_system_time,
                                self.run_started_at,
                                timestamp,
                            ))),
                            "duration_nanos": task_record
                                .started_at
                                .zip(task_record.finished_at)
                                .map(|(started_at, finished_at)| finished_at.as_nanos().saturating_sub(started_at.as_nanos()).to_string()),
                            "log_file": format!("{}.log", sanitize_file_component(task_record.name.as_str())),
                        })
                    })
                    .collect::<Vec<_>>(),
            }))?
            .as_slice(),
        )?;

        Ok(())
    }
}

fn task_plan_json(task: &Task) -> serde_json::Value {
    let run = match &task.run {
        RunSpec::Shell(command) => json!({
            "kind": "shell",
            "command": command.as_str(),
        }),
        RunSpec::Script(script) => json!({
            "kind": "script",
            "path": script.as_str(),
        }),
        RunSpec::Container(container) => json!({
            "kind": "container",
            "image": container.image.as_str(),
            "args": container.args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>(),
        }),
    };

    json!({
        "name": task.name.as_str(),
        "description": task.description.as_ref().map(|value| value.as_str()),
        "dependencies": task.dependencies.iter().map(|dependency| dependency.as_str()).collect::<Vec<_>>(),
        "run": run,
        "environment": task.environment.iter().map(|variable| {
            json!({
                "name": variable.name.as_str(),
                "value": variable.value.as_str(),
            })
        }).collect::<Vec<_>>(),
        "artifacts": task.artifacts.iter().map(|artifact| {
            json!({
                "name": artifact.name.as_str(),
                "path": artifact.path.as_str(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn render_task_log(
    run_started_system_time: SystemTime,
    run_started_at: Timestamp,
    log_lines: &[(Timestamp, ProcessOutputStream, String)],
) -> String {
    let mut rendered = String::new();
    for (timestamp, stream, line) in log_lines {
        let stream_name = match stream {
            ProcessOutputStream::Stdout => "stdout",
            ProcessOutputStream::Stderr => "stderr",
        };
        rendered.push_str(&format!(
            "[{}] {}: {}\n",
            format_iso8601(absolute_system_time(
                run_started_system_time,
                run_started_at,
                *timestamp,
            )),
            stream_name,
            line
        ));
    }
    rendered
}

fn render_events_jsonl(
    run_started_system_time: SystemTime,
    run_started_at: Timestamp,
    planned_run: &PlannedRun,
    task_events: &[TaskEventRecord],
    run_finished_at: Timestamp,
    overall_result: &str,
) -> String {
    let mut lines = Vec::new();
    lines.push(
        serde_json::to_string(&json!({
            "type": "run_started",
            "timestamp": format_iso8601(run_started_system_time),
            "requested_tasks": planned_run
                .requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>(),
        }))
        .unwrap(),
    );

    for task_event in task_events {
        match task_event {
            TaskEventRecord::Started {
                task_name,
                timestamp,
            } => lines.push(
                serde_json::to_string(&json!({
                    "type": "task_started",
                    "timestamp": format_iso8601(absolute_system_time(
                        run_started_system_time,
                        run_started_at,
                        *timestamp,
                    )),
                    "task": task_name.as_str(),
                }))
                .unwrap(),
            ),
            TaskEventRecord::Finished {
                task_name,
                timestamp,
                status,
                result,
                exit_code,
            } => lines.push(
                serde_json::to_string(&json!({
                    "type": "task_finished",
                    "timestamp": format_iso8601(absolute_system_time(
                        run_started_system_time,
                        run_started_at,
                        *timestamp,
                    )),
                    "task": task_name.as_str(),
                    "status": status.as_str(),
                    "result": result.as_str(),
                    "exit_code": exit_code,
                }))
                .unwrap(),
            ),
            TaskEventRecord::Skipped {
                task_name,
                timestamp,
            } => lines.push(
                serde_json::to_string(&json!({
                    "type": "task_skipped",
                    "timestamp": format_iso8601(absolute_system_time(
                        run_started_system_time,
                        run_started_at,
                        *timestamp,
                    )),
                    "task": task_name.as_str(),
                }))
                .unwrap(),
            ),
        }
    }

    lines.push(
        serde_json::to_string(&json!({
            "type": "run_finished",
            "timestamp": format_iso8601(absolute_system_time(
                run_started_system_time,
                run_started_at,
                run_finished_at,
            )),
            "result": overall_result,
        }))
        .unwrap(),
    );

    lines.join("\n") + "\n"
}

fn absolute_system_time(
    run_started_system_time: SystemTime,
    run_started_at: Timestamp,
    timestamp: Timestamp,
) -> SystemTime {
    let delta_nanos = timestamp
        .as_nanos()
        .saturating_sub(run_started_at.as_nanos());
    run_started_system_time
        .checked_add(Duration::from_nanos(
            u64::try_from(delta_nanos).unwrap_or(u64::MAX),
        ))
        .unwrap_or(run_started_system_time)
}

fn format_iso8601(system_time: SystemTime) -> String {
    let date_time = OffsetDateTime::from(system_time).to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        date_time.year(),
        date_time.month() as u8,
        date_time.day(),
        date_time.hour(),
        date_time.minute(),
        date_time.second()
    )
}

fn format_file_safe_iso8601(system_time: SystemTime) -> String {
    format_iso8601(system_time).replace(':', "-")
}

fn sanitize_file_component(component: &str) -> String {
    component
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '+' => character,
            _ => '_',
        })
        .collect()
}
