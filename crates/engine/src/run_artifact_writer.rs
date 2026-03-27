use crate::planned_run::PlannedRun;
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

const RUN_DIRECTORY_RETRY_LIMIT: usize = 30;
const RUN_DIRECTORY_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Writes `.nao/runs` artifacts for one executed run.
#[derive(Clone)]
pub struct RunArtifactWriter {
    pal: PalHandle,
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
    /// Final reported task outcome when available.
    pub outcome_message: Option<SharedString>,
    /// Timestamped log lines emitted by the task.
    pub log_lines: Vec<(Timestamp, ProcessOutputStream, String)>,
}

impl RunArtifactWriter {
    /// Creates a run artifact writer for one planned invocation.
    pub fn new(
        pal: PalHandle,
        recipe_path: &FilePath,
        requested_task_names: &[String],
        run_started_at: Timestamp,
        run_started_system_time: SystemTime,
    ) -> NaoResult<Self> {
        let run_root_directory = run_root_directory_for_recipe_path(recipe_path);
        pal.create_directory_all(&run_root_directory)?;
        let run_directory = reserve_run_directory(
            &*pal,
            &run_root_directory,
            requested_task_names,
            run_started_system_time,
        )?;

        Ok(Self {
            pal,
            run_directory: run_directory.clone(),
            run_started_at,
            run_started_system_time,
        })
    }

    /// Predicts the run directory for a run that starts at the provided time.
    pub fn preview_run_directory(
        recipe_path: &FilePath,
        requested_task_names: &[String],
        run_started_system_time: SystemTime,
    ) -> FilePath {
        let run_root_directory = run_root_directory_for_recipe_path(recipe_path);
        preview_run_directory_for_time(
            &run_root_directory,
            requested_task_names,
            run_started_system_time,
        )
    }

    /// Returns the run directory for this execution.
    pub fn run_directory(&self) -> FilePath {
        self.run_directory.clone()
    }

    /// Creates the run directory and writes the planned run description.
    pub fn write_plan(&self, planned_run: &PlannedRun) -> NaoResult<()> {
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

    /// Writes the initial run-start event so browsers can open the run while it is active.
    pub fn write_run_started(&self, planned_run: &PlannedRun) -> NaoResult<()> {
        self.pal.write_file(
            &self.run_directory.join("nao-events.jsonl"),
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "run_started",
                    "timestamp": format_iso8601(self.run_started_system_time),
                    "requested_tasks": planned_run
                        .requested_tasks
                        .iter()
                        .map(|task| task.as_str())
                        .collect::<Vec<_>>(),
                }))?
            )
            .as_bytes(),
        )?;
        Ok(())
    }

    /// Appends a task start event.
    pub fn append_task_started(&self, task_name: &str, timestamp: Timestamp) -> NaoResult<()> {
        self.append_event_json(&json!({
            "type": "task_started",
            "timestamp": format_iso8601(absolute_system_time(
                self.run_started_system_time,
                self.run_started_at,
                timestamp,
            )),
            "task": task_name,
        }))
    }

    /// Appends a task finish event.
    pub fn append_task_finished(&self, task_record: &TaskArtifactRecord) -> NaoResult<()> {
        let timestamp = task_record.finished_at.unwrap_or(self.run_started_at);
        self.append_event_json(&json!({
            "type": "task_finished",
            "timestamp": format_iso8601(absolute_system_time(
                self.run_started_system_time,
                self.run_started_at,
                timestamp,
            )),
            "task": task_record.name.as_str(),
            "status": task_record.status.as_str(),
            "result": task_record.result.as_str(),
            "exit_code": task_record.exit_code,
            "outcome_message": task_record.outcome_message.as_ref().map(|value| value.as_str()),
            "duration_nanos": task_record
                .started_at
                .zip(task_record.finished_at)
                .map(|(started_at, finished_at)| finished_at.as_nanos().saturating_sub(started_at.as_nanos()).to_string()),
        }))
    }

    /// Appends a task skipped event.
    pub fn append_task_skipped(&self, task_name: &str, timestamp: Timestamp) -> NaoResult<()> {
        self.append_event_json(&json!({
            "type": "task_skipped",
            "timestamp": format_iso8601(absolute_system_time(
                self.run_started_system_time,
                self.run_started_at,
                timestamp,
            )),
            "task": task_name,
        }))
    }

    /// Appends a rendered task log line to the per-task log file.
    pub fn append_task_log_line(
        &self,
        task_name: &SharedString,
        timestamp: Timestamp,
        stream: ProcessOutputStream,
        line: &str,
    ) -> NaoResult<()> {
        let stream_name = match stream {
            ProcessOutputStream::Stdout => "stdout",
            ProcessOutputStream::Stderr => "stderr",
        };
        self.pal.append_file(
            &self.task_log_path(task_name.as_str()),
            format!(
                "[{}] {}: {}\n",
                format_iso8601(absolute_system_time(
                    self.run_started_system_time,
                    self.run_started_at,
                    timestamp,
                )),
                stream_name,
                line
            )
            .as_bytes(),
        )?;
        Ok(())
    }

    /// Writes task logs, event lines, and the final run summary.
    pub fn write_completion(
        &self,
        planned_run: &PlannedRun,
        task_records: &[TaskArtifactRecord],
        run_finished_at: Timestamp,
        overall_result: &str,
        failure_message: Option<&str>,
    ) -> NaoResult<()> {
        for task_record in task_records {
            self.ensure_task_log_file(task_record)?;
        }

        self.append_event_json(&json!({
            "type": "run_finished",
            "timestamp": format_iso8601(absolute_system_time(
                self.run_started_system_time,
                self.run_started_at,
                run_finished_at,
            )),
            "result": overall_result,
        }))?;

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
                            "outcome_message": task_record.outcome_message.as_ref().map(|value| value.as_str()),
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

    fn task_log_path(&self, task_name: &str) -> FilePath {
        self.run_directory.join(task_log_file_name(task_name))
    }

    fn append_event_json(&self, value: &serde_json::Value) -> NaoResult<()> {
        self.pal.append_file(
            &self.run_directory.join("nao-events.jsonl"),
            format!("{}\n", serde_json::to_string(value)?).as_bytes(),
        )?;
        Ok(())
    }

    fn ensure_task_log_file(&self, task_record: &TaskArtifactRecord) -> NaoResult<()> {
        let path = self.task_log_path(task_record.name.as_str());
        if !self.pal.file_exists(&path)? {
            self.pal.write_file(
                &path,
                render_task_log(
                    self.run_started_system_time,
                    self.run_started_at,
                    &task_record.log_lines,
                )
                .as_bytes(),
            )?;
        }
        Ok(())
    }
}

/// Returns the `.nao/runs` directory associated with a recipe path.
pub fn run_root_directory_for_recipe_path(recipe_path: &FilePath) -> FilePath {
    let recipe_directory = recipe_path.parent().unwrap_or_else(|| FilePath::from("."));
    let recipe_directory = if recipe_directory.as_str().is_empty() {
        FilePath::from(".")
    } else {
        recipe_directory
    };
    if recipe_path.file_name() == Some("nao.kdl") && recipe_directory.file_name() == Some(".nao") {
        return recipe_directory.join("runs").normalize();
    }

    recipe_directory.join(".nao").join("runs").normalize()
}

fn reserve_run_directory(
    pal: &dyn nao_pal::pal::Pal,
    run_root_directory: &FilePath,
    requested_task_names: &[String],
    run_started_system_time: SystemTime,
) -> NaoResult<FilePath> {
    for attempt in 0..RUN_DIRECTORY_RETRY_LIMIT {
        let candidate_time =
            run_started_system_time + Duration::from_secs(u64::try_from(attempt).unwrap_or(0));
        let run_directory = preview_run_directory_for_time(
            run_root_directory,
            requested_task_names,
            candidate_time,
        );
        if pal.create_directory(&run_directory)? {
            return Ok(run_directory);
        }
        if attempt + 1 < RUN_DIRECTORY_RETRY_LIMIT {
            pal.sleep(RUN_DIRECTORY_RETRY_DELAY);
        }
    }

    Err(nao_base::err!(
        "Unable to reserve a unique run directory after {} attempts",
        RUN_DIRECTORY_RETRY_LIMIT
    ))
}

fn preview_run_directory_for_time(
    run_root_directory: &FilePath,
    requested_task_names: &[String],
    run_started_system_time: SystemTime,
) -> FilePath {
    let run_directory_name = format!(
        "{}-{}",
        format_file_safe_iso8601(run_started_system_time),
        sanitize_file_component(&requested_task_names.join("+"))
    );
    run_root_directory.join(run_directory_name).normalize()
}

/// Returns the persisted task log file name for a task.
pub fn task_log_file_name(task_name: &str) -> String {
    format!("{}.log", sanitize_file_component(task_name))
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
        RunSpec::Compose(compose) => json!({
            "kind": "compose",
            "directory": compose.directory.as_str(),
            "service": compose.service.as_str(),
            "args": compose.args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>(),
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
