use nao_base::file_path::FilePath;
use nao_base::result::{NaoResult, ResultExt};
use nao_base::shared_string::SharedString;
use nao_engine::run_artifact_writer::run_root_directory_for_recipe_path;
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
    /// Final reported task outcome when available.
    pub outcome_message: Option<SharedString>,
    /// Task duration in nanoseconds when known.
    pub duration_nanos: Option<u128>,
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
    /// Final reported task outcome when applicable.
    pub outcome_message: Option<SharedString>,
    /// Task duration in nanoseconds when applicable.
    pub duration_nanos: Option<u128>,
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
    /// Total run duration in nanoseconds when known.
    pub duration_nanos: Option<u128>,
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
    duration_nanos: Option<SharedString>,
}

#[derive(Debug, Clone, Deserialize)]
struct SummaryTaskFile {
    name: SharedString,
    status: SharedString,
    result: SharedString,
    exit_code: Option<i32>,
    outcome_message: Option<SharedString>,
    duration_nanos: Option<SharedString>,
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
    #[serde(default)]
    outcome_message: Option<SharedString>,
    #[serde(default)]
    duration_nanos: Option<SharedString>,
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
    let failure_message = summary
        .as_ref()
        .and_then(|summary| summary.failure_message.clone());
    let duration_nanos = summary
        .as_ref()
        .and_then(|summary| summary.run.duration_nanos.as_ref())
        .and_then(|duration| duration.as_str().parse::<u128>().ok());

    Ok(RunDetailRecord {
        run_directory: run_directory.clone(),
        run_id: run_id_from_directory(run_directory),
        requested_tasks,
        result,
        duration_nanos,
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
        .map(strip_log_metadata_prefix)
        .collect::<Vec<SharedString>>())
}

fn strip_log_metadata_prefix(line: &str) -> SharedString {
    let Some(after_timestamp) = line
        .strip_prefix('[')
        .and_then(|line| line.split_once("] "))
    else {
        return SharedString::from(line);
    };
    let (_, remainder) = after_timestamp;
    let Some((stream_name, message)) = remainder.split_once(": ") else {
        return SharedString::from(remainder);
    };
    if matches!(stream_name, "stdout" | "stderr") {
        SharedString::from(message)
    } else {
        SharedString::from(remainder)
    }
}

fn load_run_summary(pal: &dyn Pal, run_directory: &FilePath) -> NaoResult<RunSummaryRecord> {
    if let Some(summary) = read_summary_file(pal, run_directory)? {
        return Ok(RunSummaryRecord {
            run_directory: run_directory.clone(),
            run_id: run_id_from_directory(run_directory),
            requested_tasks: summary.run.requested_tasks,
            result: summary.result,
            failure_message: summary.failure_message,
            task_count: summary.tasks.len(),
        });
    }

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
                outcome_message: task.outcome_message.clone(),
                duration_nanos: task
                    .duration_nanos
                    .as_ref()
                    .and_then(|duration| duration.as_str().parse::<u128>().ok()),
                log_file: FilePath::from(task.log_file.clone()),
            })
            .collect();
    }

    let mut tasks = plan
        .map(|plan| {
            plan.tasks
                .iter()
                .map(|task| RunTaskRecord {
                    name: task.name.clone(),
                    status: SharedString::from("pending"),
                    result: SharedString::from("pending"),
                    exit_code: None,
                    outcome_message: None,
                    duration_nanos: None,
                    log_file: FilePath::from(format!(
                        "{}.log",
                        sanitize_task_name(task.name.as_str())
                    )),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut task_indexes = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.name.clone(), index))
        .collect::<BTreeMap<_, _>>();

    for event in events {
        let Some(task_name) = &event.task_name else {
            continue;
        };
        let task_index = if let Some(task_index) = task_indexes.get(task_name).copied() {
            task_index
        } else {
            let task_index = tasks.len();
            tasks.push(RunTaskRecord {
                name: task_name.clone(),
                status: SharedString::from("pending"),
                result: SharedString::from("pending"),
                exit_code: None,
                outcome_message: None,
                duration_nanos: None,
                log_file: FilePath::from(format!("{}.log", sanitize_task_name(task_name.as_str()))),
            });
            task_indexes.insert(task_name.clone(), task_index);
            task_index
        };
        let task = &mut tasks[task_index];
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
                task.outcome_message = event.outcome_message.clone();
                task.duration_nanos = event.duration_nanos;
            }
            "task_skipped" => {
                task.status = SharedString::from("skipped");
                task.result = SharedString::from("skipped");
            }
            _ => {}
        }
    }

    tasks
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
                outcome_message: event.outcome_message,
                duration_nanos: event
                    .duration_nanos
                    .as_ref()
                    .and_then(|duration| duration.as_str().parse::<u128>().ok()),
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
    run_root_directory_for_recipe_path(recipe_path)
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
    fn discovers_runs_from_summary_without_loading_full_detail() {
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
        pal.clear_effects();

        let runs = discover_runs(&pal, &FilePath::from("nao.kdl")).unwrap();

        assert_eq!(runs.len(), 1);
        pal.verify_effects(expect![[r#"
READ FILE: .nao/runs/2026-03-19T12-00-00Z-test/nao-summary.json
"#]]);
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
    fn loads_persisted_task_outcomes_from_summary_and_events() {
        let pal = PalMock::new();
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/nao-summary.json",
            r#"{
              "result":"completed",
              "failure_message":null,
              "run":{"requested_tasks":["test"],"duration_nanos":"10"},
              "tasks":[
                {
                  "name":"test",
                  "status":"completed",
                  "result":"success",
                  "exit_code":0,
                  "outcome_message":"30 tests succeeded",
                  "duration_nanos":"10",
                  "log_file":"test.log"
                }
              ]
            }"#,
        );
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/nao-events.jsonl",
            "{\"type\":\"task_finished\",\"timestamp\":\"2026-03-19T12:00:01Z\",\"task\":\"test\",\"status\":\"completed\",\"result\":\"success\",\"exit_code\":0,\"outcome_message\":\"30 tests succeeded\",\"duration_nanos\":\"10\"}\n",
        );

        let detail =
            load_run_detail(&pal, &FilePath::from(".nao/runs/2026-03-19T12-00-00Z-test")).unwrap();

        assert_eq!(
            detail.tasks[0]
                .outcome_message
                .as_ref()
                .map(|value| value.as_str()),
            Some("30 tests succeeded")
        );
        assert_eq!(
            detail.events[0]
                .outcome_message
                .as_ref()
                .map(|value| value.as_str()),
            Some("30 tests succeeded")
        );
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
            r#"compiling
linking"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn preserves_plain_log_lines_without_metadata_prefix() {
        let pal = PalMock::new();
        pal.set_file(
            ".nao/runs/2026-03-19T12-00-00Z-test/build.log",
            "plain line\nstderr but no timestamp prefix\n",
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
            r#"plain line
stderr but no timestamp prefix"#
        ]
        .assert_eq(&rendered);
    }
}
