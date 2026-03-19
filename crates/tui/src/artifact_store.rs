use nao_base::file_path::FilePath;
use nao_base::result::{NaoResult, ResultExt};
use nao_base::shared_string::SharedString;
use nao_pal::pal::Pal;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Summarizes one discovered run directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummaryRecord {
    /// Directory that stores the run artifacts.
    pub run_directory: FilePath,
    /// User-facing run identifier derived from the directory name.
    pub run_id: SharedString,
    /// Requested top-level tasks.
    pub requested_tasks: Vec<SharedString>,
    /// Current or final run result.
    pub result: SharedString,
    /// Failure summary when the run failed.
    pub failure_message: Option<SharedString>,
    /// Number of tasks included in the run.
    pub task_count: usize,
}

/// Describes one task within a run detail view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTaskRecord {
    /// Task name.
    pub name: SharedString,
    /// Current or final task status.
    pub status: SharedString,
    /// Current or final task result.
    pub result: SharedString,
    /// Process exit code when known.
    pub exit_code: Option<i32>,
    /// Task log path relative to the run directory.
    pub log_file: FilePath,
}

/// Describes one event from `nao-events.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEventRecord {
    /// Event type.
    pub event_type: SharedString,
    /// Event timestamp.
    pub timestamp: SharedString,
    /// Task name when applicable.
    pub task_name: Option<SharedString>,
    /// Task status when applicable.
    pub status: Option<SharedString>,
    /// Run or task result when applicable.
    pub result: Option<SharedString>,
    /// Exit code when applicable.
    pub exit_code: Option<i32>,
}

/// Fully parsed data for one run detail screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDetailRecord {
    /// Directory that stores the run artifacts.
    pub run_directory: FilePath,
    /// User-facing run identifier derived from the directory name.
    pub run_id: SharedString,
    /// Requested top-level tasks.
    pub requested_tasks: Vec<SharedString>,
    /// Current or final run result.
    pub result: SharedString,
    /// Failure summary when the run failed.
    pub failure_message: Option<SharedString>,
    /// Tasks shown in the task list.
    pub tasks: Vec<RunTaskRecord>,
    /// Events shown in the event list.
    pub events: Vec<RunEventRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct SummaryFile {
    result: SharedString,
    failure_message: Option<SharedString>,
    run: SummaryRunFile,
    tasks: Vec<SummaryTaskFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct SummaryRunFile {
    requested_tasks: Vec<SharedString>,
}

#[derive(Debug, Clone, Deserialize)]
struct SummaryTaskFile {
    name: SharedString,
    status: SharedString,
    result: SharedString,
    exit_code: Option<i32>,
    log_file: SharedString,
}

#[derive(Debug, Clone, Deserialize)]
struct PlanFile {
    requested_tasks: Vec<SharedString>,
    tasks: Vec<PlanTaskFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlanTaskFile {
    name: SharedString,
}

#[derive(Debug, Clone, Deserialize)]
struct EventFile {
    #[serde(rename = "type")]
    event_type: SharedString,
    timestamp: SharedString,
    #[serde(default)]
    task: Option<SharedString>,
    #[serde(default)]
    status: Option<SharedString>,
    #[serde(default)]
    result: Option<SharedString>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default, rename = "requested_tasks")]
    _requested_tasks: Vec<SharedString>,
}

/// Discovers `.nao/runs` for the recipe and returns them newest-first.
pub fn discover_runs(pal: &dyn Pal, recipe_path: &FilePath) -> NaoResult<Vec<RunSummaryRecord>> {
    let run_root = run_root_directory(recipe_path);
    let mut run_directories = BTreeMap::<SharedString, FilePath>::new();
    let entries = match pal.walk_directory(&run_root, &["**/*".to_owned()]) {
        Ok(entries) => entries,
        Err(_) => return Ok(Vec::new()),
    };

    for entry in entries {
        let entry = entry?;
        let Ok(relative_path) = entry.as_path().strip_prefix(run_root.as_path()) else {
            continue;
        };
        let Some(run_directory_name) = relative_path.components().next() else {
            continue;
        };
        let run_directory_name = run_directory_name.as_os_str().to_string_lossy().to_string();
        run_directories
            .entry(SharedString::from(run_directory_name.as_str()))
            .or_insert_with(|| run_root.join(run_directory_name));
    }

    let mut runs = run_directories
        .into_values()
        .map(|run_directory| load_run_summary(pal, &run_directory))
        .collect::<NaoResult<Vec<_>>>()?;
    runs.sort_by(|left, right| right.run_id.cmp(&left.run_id));
    Ok(runs)
}

/// Loads the full detail view model for one run directory.
pub fn load_run_detail(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<RunDetailRecord> {
    let summary = read_summary_file(pal, run_directory)?;
    let plan = read_plan_file(pal, run_directory)?;
    let events = read_events_file(pal, run_directory)?;
    let tasks = build_task_records(summary.as_ref(), plan.as_ref(), &events);
    let requested_tasks = summary
        .as_ref()
        .map(|summary| summary.run.requested_tasks.clone())
        .or_else(|| plan.as_ref().map(|plan| plan.requested_tasks.clone()))
        .unwrap_or_default();
    let result = summary
        .as_ref()
        .map(|summary| summary.result.clone())
        .unwrap_or_else(|| live_result_from_events(&events));
    let failure_message = summary.and_then(|summary| summary.failure_message);

    Ok(RunDetailRecord {
        run_directory: run_directory.clone(),
        run_id: run_id_from_directory(run_directory),
        requested_tasks,
        result,
        failure_message,
        tasks,
        events,
    })
}

/// Reads one task log and returns display lines.
pub fn load_task_log_lines(
    pal: &dyn Pal,
    run_directory: &FilePath,
    log_file: &FilePath,
) -> NaoResult<Vec<SharedString>> {
    let full_path = run_directory.join(log_file.as_str());
    if !pal.file_exists(&full_path)? {
        return Ok(Vec::new());
    }
    let content = pal.read_file_to_string(&full_path)?;
    Ok(content
        .lines()
        .map(SharedString::from)
        .collect::<Vec<SharedString>>())
}

fn load_run_summary(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<RunSummaryRecord> {
    let detail = load_run_detail(pal, run_directory)?;
    Ok(RunSummaryRecord {
        run_directory: detail.run_directory,
        run_id: detail.run_id,
        requested_tasks: detail.requested_tasks,
        result: detail.result,
        failure_message: detail.failure_message,
        task_count: detail.tasks.len(),
    })
}

fn build_task_records(
    summary: Option<&SummaryFile>,
    plan: Option<&PlanFile>,
    events: &[RunEventRecord],
) -> Vec<RunTaskRecord> {
    if let Some(summary) = summary {
        return summary
            .tasks
            .iter()
            .map(|task| RunTaskRecord {
                name: task.name.clone(),
                status: task.status.clone(),
                result: task.result.clone(),
                exit_code: task.exit_code,
                log_file: FilePath::from(task.log_file.clone()),
            })
            .collect();
    }

    let mut tasks = plan
        .map(|plan| {
            plan.tasks
                .iter()
                .map(|task| {
                    (
                        task.name.clone(),
                        RunTaskRecord {
                            name: task.name.clone(),
                            status: SharedString::from("pending"),
                            result: SharedString::from("pending"),
                            exit_code: None,
                            log_file: FilePath::from(format!(
                                "{}.log",
                                sanitize_task_name(task.name.as_str())
                            )),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    for event in events {
        let Some(task_name) = &event.task_name else {
            continue;
        };
        let task = tasks
            .entry(task_name.clone())
            .or_insert_with(|| RunTaskRecord {
                name: task_name.clone(),
                status: SharedString::from("pending"),
                result: SharedString::from("pending"),
                exit_code: None,
                log_file: FilePath::from(format!("{}.log", sanitize_task_name(task_name.as_str()))),
            });
        match event.event_type.as_str() {
            "task_started" => {
                task.status = SharedString::from("running");
                task.result = SharedString::from("running");
            }
            "task_finished" => {
                task.status = event
                    .status
                    .clone()
                    .unwrap_or_else(|| SharedString::from("completed"));
                task.result = event.result.clone().unwrap_or_else(|| task.status.clone());
                task.exit_code = event.exit_code;
            }
            "task_skipped" => {
                task.status = SharedString::from("skipped");
                task.result = SharedString::from("skipped");
            }
            _ => {}
        }
    }

    tasks.into_values().collect()
}

fn read_summary_file(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<Option<SummaryFile>> {
    let summary_path = run_directory.join("nao-summary.json");
    if !pal.file_exists(&summary_path)? {
        return Ok(None);
    }
    let content = pal.read_file_to_string(&summary_path)?;
    Ok(Some(serde_json::from_str(content.as_str()).with_context(
        || format!("Unable to parse '{}'", summary_path),
    )?))
}

fn read_plan_file(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<Option<PlanFile>> {
    let plan_path = run_directory.join("nao-plan.json");
    if !pal.file_exists(&plan_path)? {
        return Ok(None);
    }
    let content = pal.read_file_to_string(&plan_path)?;
    Ok(Some(serde_json::from_str(content.as_str()).with_context(
        || format!("Unable to parse '{}'", plan_path),
    )?))
}

fn read_events_file(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<Vec<RunEventRecord>> {
    let events_path = run_directory.join("nao-events.jsonl");
    if !pal.file_exists(&events_path)? {
        return Ok(Vec::new());
    }
    let content = pal.read_file_to_string(&events_path)?;
    let mut events = Vec::new();
    let ends_with_newline = content.as_str().ends_with('\n');

    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<EventFile>(line) {
            Ok(event) => events.push(RunEventRecord {
                event_type: event.event_type,
                timestamp: event.timestamp,
                task_name: event.task,
                status: event.status,
                result: event.result,
                exit_code: event.exit_code,
            }),
            Err(error) => {
                let is_last_line = index + 1 == content.lines().count();
                if is_last_line && !ends_with_newline {
                    break;
                }
                return Err(error).with_context(|| format!("Unable to parse '{}'", events_path));
            }
        }
    }

    Ok(events)
}

fn live_result_from_events(events: &[RunEventRecord]) -> SharedString {
    for event in events.iter().rev() {
        if event.event_type == "run_finished" {
            return event
                .result
                .clone()
                .unwrap_or_else(|| SharedString::from("completed"));
        }
    }
    if events.is_empty() {
        SharedString::from("unknown")
    } else {
        SharedString::from("running")
    }
}

fn run_root_directory(recipe_path: &FilePath) -> FilePath {
    let recipe_directory = recipe_path.parent().unwrap_or_else(|| FilePath::from("."));
    let recipe_directory = if recipe_directory.as_str().is_empty() {
        FilePath::from(".")
    } else {
        recipe_directory
    };
    recipe_directory.join(".nao").join("runs").normalize()
}

fn run_id_from_directory(run_directory: &FilePath) -> SharedString {
    run_directory
        .file_name()
        .map(SharedString::from)
        .unwrap_or_else(SharedString::empty)
}

fn sanitize_task_name(task_name: &str) -> SharedString {
    SharedString::from(
        task_name
            .chars()
            .map(|character| match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '+' => character,
                _ => '_',
            })
            .collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::{discover_runs, load_run_detail, load_task_log_lines};
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_pal::pal_mock::PalMock;

    #[test]
    fn discovers_runs_and_sorts_latest_first() {
        let pal = PalMock::new();
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/nao-summary.json",
            r#"{
              "result":"completed",
              "failure_message":null,
              "run":{"requested_tasks":["test"]},
              "tasks":[]
            }"#,
        );
        pal.set_file(
            ".nao/runs/2026-03-19T11-00-00Z-build/nao-summary.json",
            r#"{
              "result":"failed",
              "failure_message":"boom",
              "run":{"requested_tasks":["build"]},
              "tasks":[]
            }"#,
        );

        let runs = discover_runs(&pal, &FilePath::from("nao.kdl")).unwrap();
        let rendered = runs
            .iter()
            .map(|run| format!("{} {}", run.run_id.as_str(), run.result.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        expect![
            r#"2026-03-19T12-00-00Z-test completed
2026-03-19T11-00-00Z-build failed"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn tolerates_partially_written_events_for_active_run() {
        let pal = PalMock::new();
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/nao-plan.json",
            r#"{
              "requested_tasks":["test"],
              "tasks":[{"name":"build"},{"name":"test"}]
            }"#,
        );
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/nao-events.jsonl",
            concat!(
                "{\"type\":\"run_started\",\"timestamp\":\"2026-03-19T12:00:00Z\",\"requested_tasks\":[\"test\"]}\n",
                "{\"type\":\"task_started\",\"timestamp\":\"2026-03-19T12:00:01Z\",\"task\":\"build\"}\n",
                "{\"type\":\"task_finished\""
            ),
        );

        let detail =
            load_run_detail(&pal, &FilePath::from(".nao/runs/2026-03-19T12-00-00Z-test")).unwrap();
        let rendered = detail
            .tasks
            .iter()
            .map(|task| format!("{} {}", task.name.as_str(), task.status.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        expect![
            r#"build running
test pending"#
        ]
        .assert_eq(&rendered);
        assert_eq!(detail.result, "running");
    }

    #[test]
    fn loads_task_logs() {
        let pal = PalMock::new();
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/build.log",
            "[2026-03-19T12:00:01Z] stdout: compiling\n[2026-03-19T12:00:02Z] stdout: linking\n",
        );

        let lines = load_task_log_lines(
            &pal,
            &FilePath::from(".nao/runs/2026-03-19T12-00-00Z-test"),
            &FilePath::from("build.log"),
        )
        .unwrap();

        let rendered = lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        expect![
            r#"[2026-03-19T12:00:01Z] stdout: compiling
[2026-03-19T12:00:02Z] stdout: linking"#
        ]
        .assert_eq(&rendered);
    }
}
