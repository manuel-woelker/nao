use super::{ActiveRunHandle, App, Focus, Screen, pretty_duration, render_task_state_emoji};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::cancellation_token::CancellationToken;
use nao_pal::pal::PalHandle;
use nao_pal::pal_mock::PalMock;
use nao_pal::process_command::ProcessCommand;
use nao_pal::process_event::ProcessEvent;
use nao_pal::process_exited_event::ProcessExitedEvent;
use nao_pal::process_result::ProcessResult;
use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
use std::mem;
use std::sync::mpsc;
use std::time::SystemTime;

fn test_app() -> App {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }

              task "test" description="Test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: "./scripts/build.sh".into(),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(1),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(3),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(3),
            exit_code: Some(0),
        },
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: "./scripts/test.sh".into(),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(4),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(5),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(6),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(3),
            finished_at: Timestamp::new(6),
            exit_code: Some(0),
        },
    );
    App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap()
}

fn test_app_with_pal() -> (App, PalMock) {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }

              task "test" description="Test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: "./scripts/build.sh".into(),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(1),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(3),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(3),
            exit_code: Some(0),
        },
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: "./scripts/test.sh".into(),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(4),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(5),
                stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(6),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(3),
            finished_at: Timestamp::new(6),
            exit_code: Some(0),
        },
    );
    (
        App::new(PalHandle::new(pal.clone()), FilePath::from("nao.kdl")).unwrap(),
        pal,
    )
}

fn test_app_with_active_run() -> (App, PalMock) {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }

              task "test" description="Test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
    );
    pal.set_file(
        ".nao/runs/1970-01-01T00-00-00Z-test/nao-plan.json",
        r#"{
              "requested_tasks":["test"],
              "tasks":[{"name":"build"},{"name":"test"}]
            }"#,
    );
    pal.set_file(
        ".nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl",
        concat!(
            "{\"type\":\"run_started\",\"timestamp\":\"1970-01-01T00:00:00Z\",\"requested_tasks\":[\"test\"]}\n",
            "{\"type\":\"task_started\",\"timestamp\":\"1970-01-01T00:00:01Z\",\"task\":\"build\"}\n"
        ),
    );
    pal.set_file(
        ".nao/runs/1970-01-01T00-00-00Z-test/build.log",
        "[1970-01-01T00:00:01Z] stdout: compiling\n",
    );

    let mut app = App::new(PalHandle::new(pal.clone()), FilePath::from("nao.kdl")).unwrap();
    app.open_run(&FilePath::from(".nao/runs/1970-01-01T00-00-00Z-test"))
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    mem::forget(sender);
    app.active_run = Some(ActiveRunHandle {
        run_directory: FilePath::from(".nao/runs/1970-01-01T00-00-00Z-test"),
        requested_goal_tasks: vec![SharedString::from("test")],
        cancellation_token: CancellationToken::new(),
        receiver,
    });
    pal.clear_effects();

    (app, pal)
}

#[test]
fn launcher_keys_toggle_goals_and_switch_screens() {
    let mut app = test_app();
    app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_event(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.screen, Screen::RunHistory);
    assert_eq!(app.focus, Focus::HistoryRuns);
    assert!(app.selected_goals.contains("test"));
}

#[test]
fn launcher_defaults_run_target_to_selected_task() {
    let mut app = test_app();
    app.selected_task_index = 1;

    assert_eq!(app.launcher_goal_tasks(), vec!["test".to_owned()]);
}

#[test]
fn launcher_prefers_explicit_goal_selection() {
    let mut app = test_app();
    app.selected_task_index = 0;
    app.selected_goals.insert("test".into());

    assert_eq!(app.launcher_goal_tasks(), vec!["test".to_owned()]);
}

#[test]
fn launching_keeps_the_launcher_screen_active() {
    let mut app = test_app();
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.screen, Screen::Launcher);
    assert!(app.active_run.is_some());
}

#[test]
fn ctrl_r_requests_restart_for_active_run() {
    let (mut app, _) = test_app_with_active_run();
    app.selected_goals.insert("build".into());

    app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    let active_run = app.active_run.as_ref().unwrap();
    assert!(active_run.cancellation_token.is_cancelled());
    assert_eq!(
        app.pending_restart_goal_tasks,
        Some(vec![SharedString::from("test")])
    );
    assert_eq!(app.status_message.as_deref(), Some("restart requested"));
}

#[test]
fn ctrl_r_without_restart_target_reports_status() {
    let mut app = test_app();

    app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(
        app.status_message.as_deref(),
        Some("no run is available to restart")
    );
}

#[test]
fn ctrl_r_restarts_last_goal_instead_of_current_launcher_selection() {
    let mut app = test_app();
    app.last_run_goal_tasks = vec![SharedString::from("build")];
    app.selected_task_index = 1;

    app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(
        app.active_run
            .as_ref()
            .map(|active_run| active_run.requested_goal_tasks.clone()),
        Some(vec![SharedString::from("build")])
    );
    assert_eq!(app.status_message.as_deref(), Some("run restarted"));
}

#[test]
fn ctrl_r_is_global_across_tui_screens() {
    for screen in [Screen::Launcher, Screen::RunDetail, Screen::RunHistory] {
        let mut app = test_app();
        app.screen = screen;
        app.last_run_goal_tasks = vec![SharedString::from("build")];

        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .unwrap();

        assert_eq!(
            app.active_run
                .as_ref()
                .map(|active_run| active_run.requested_goal_tasks.clone()),
            Some(vec![SharedString::from("build")])
        );
    }
}

#[test]
fn tab_cycles_run_detail_focus() {
    let mut app = test_app();
    app.screen = Screen::RunDetail;
    app.focus = Focus::DetailTasks;

    app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, Focus::DetailOutput);

    app.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.focus, Focus::DetailTasks);
}

#[test]
fn hotkey_two_defaults_run_detail_to_output_focus() {
    let mut app = test_app();

    app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.screen, Screen::RunDetail);
    assert_eq!(app.focus, Focus::DetailOutput);
}

#[test]
fn launcher_shows_failed_task_output_for_failed_runs() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }
            }
            "#,
    );
    pal.set_file(
        ".nao/runs/2026-03-20T10-00-00Z-build/nao-summary.json",
        r#"{
              "result":"failed",
              "failure_message":"boom",
              "run":{"requested_tasks":["build"],"duration_nanos":"10"},
              "tasks":[
                {
                  "name":"build",
                  "status":"failed",
                  "result":"failed",
                  "exit_code":1,
                  "outcome_message":"12 files checked",
                  "duration_nanos":"10",
                  "log_file":"build.log"
                }
              ]
            }"#,
    );
    pal.set_file(
        ".nao/runs/2026-03-20T10-00-00Z-build/build.log",
        "[2026-03-20T10:00:01Z] stderr: compile failed\n",
    );

    let mut app = App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap();
    app.open_run(&FilePath::from(".nao/runs/2026-03-20T10-00-00Z-build"))
        .unwrap();

    assert!(app.show_launcher_failure_output());
    assert_eq!(app.launcher_failed_task_name.as_deref(), Some("build"));
    assert_eq!(
        app.launcher_failed_task_log_lines,
        vec![SharedString::from("compile failed")]
    );
    assert_eq!(
        app.run_detail
            .as_ref()
            .and_then(|detail| detail.tasks.first())
            .and_then(|task| task.outcome_message.as_ref())
            .map(|value| value.as_str()),
        Some("12 files checked")
    );
}

#[test]
fn launcher_progress_includes_task_outcomes() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "test" description="Test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
    );
    pal.set_file(
        ".nao/runs/2026-03-20T10-00-00Z-test/nao-summary.json",
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
                  "outcome_message":"30 tests passed",
                  "duration_nanos":"10",
                  "log_file":"test.log"
                }
              ]
            }"#,
    );

    let mut app = App::new(PalHandle::new(pal), FilePath::from("nao.kdl")).unwrap();
    app.launched_run_in_session = true;
    app.open_run(&FilePath::from(".nao/runs/2026-03-20T10-00-00Z-test"))
        .unwrap();

    let rendered = app
        .render_launcher_progress_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("30 tests passed"));
}

#[test]
fn refresh_does_not_reread_completed_open_run() {
    let (mut app, pal) = test_app_with_pal();
    pal.set_file(
        ".nao/runs/2026-03-20T10-00-00Z-build/nao-summary.json",
        r#"{
              "result":"completed",
              "failure_message":null,
              "run":{"requested_tasks":["build"],"duration_nanos":"10"},
              "tasks":[
                {
                  "name":"build",
                  "status":"completed",
                  "result":"success",
                  "exit_code":0,
                  "outcome_message":null,
                  "duration_nanos":"10",
                  "log_file":"build.log"
                }
              ]
            }"#,
    );
    pal.set_file(
        ".nao/runs/2026-03-20T10-00-00Z-build/build.log",
        "[2026-03-20T10:00:01Z] stdout: done\n",
    );
    app.open_run(&FilePath::from(".nao/runs/2026-03-20T10-00-00Z-build"))
        .unwrap();
    pal.clear_effects();

    app.refresh().unwrap();

    pal.verify_effects(expect_test::expect![""]);
}

#[test]
fn refresh_skips_active_run_rereads_on_non_poll_ticks() {
    let (mut app, pal) = test_app_with_active_run();

    app.refresh().unwrap();

    pal.verify_effects(expect_test::expect![""]);
}

#[test]
fn refresh_polls_only_selected_log_before_detail_reload() {
    let (mut app, pal) = test_app_with_active_run();

    app.refresh().unwrap();
    app.refresh().unwrap();

    pal.verify_effects(expect_test::expect![[r#"
READ FILE: .nao/runs/1970-01-01T00-00-00Z-test/build.log
"#]]);
}

#[test]
fn refresh_reloads_active_run_detail_on_slower_interval() {
    let (mut app, pal) = test_app_with_active_run();

    app.refresh().unwrap();
    app.refresh().unwrap();
    app.refresh().unwrap();
    app.refresh().unwrap();

    pal.verify_effects(expect_test::expect![[r#"
READ FILE: .nao/runs/1970-01-01T00-00-00Z-test/build.log
READ FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-plan.json
READ FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl
READ FILE: .nao/runs/1970-01-01T00-00-00Z-test/build.log
"#]]);
}

#[test]
fn refresh_skips_selected_log_polling_when_auto_follow_is_disabled() {
    let (mut app, pal) = test_app_with_active_run();
    app.auto_follow_log = false;

    app.refresh().unwrap();
    app.refresh().unwrap();

    pal.verify_effects(expect_test::expect![""]);
}

#[test]
fn refresh_uses_completed_run_directory_from_engine_result() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              task "build" description="Build" {
                run script="./scripts/build.sh"
              }
            }
            "#,
    );
    let mut app = App::new(PalHandle::new(pal.clone()), FilePath::from("nao.kdl")).unwrap();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(nao_engine::RunExecutionResult {
            output: SharedString::empty(),
            goal_tasks: vec![SharedString::from("build")],
            total_task_count: 1,
            duration_nanos: 0,
            run_directory: FilePath::from(".nao/runs/1970-01-01T00-00-01Z-build"),
            task_results: Vec::new(),
            goal_outcome_message: None,
            status: nao_engine::RunStatus::Completed,
        }))
        .unwrap();
    app.active_run = Some(ActiveRunHandle {
        run_directory: FilePath::from(".nao/runs/1970-01-01T00-00-00Z-build"),
        requested_goal_tasks: vec![SharedString::from("build")],
        cancellation_token: CancellationToken::new(),
        receiver,
    });
    pal.set_file(
        ".nao/runs/1970-01-01T00-00-01Z-build/nao-summary.json",
        r#"{
              "result":"completed",
              "failure_message":null,
              "run":{"requested_tasks":["build"],"duration_nanos":"0"},
              "tasks":[]
            }"#,
    );
    pal.clear_effects();

    app.refresh().unwrap();

    assert_eq!(
        app.open_run_directory,
        Some(FilePath::from(".nao/runs/1970-01-01T00-00-01Z-build"))
    );
}

#[test]
fn renders_task_state_emojis() {
    assert_eq!(render_task_state_emoji("pending", 0), "⚪");
    assert_eq!(render_task_state_emoji("running", 0), "⠋ ");
    assert_eq!(render_task_state_emoji("running", 1), "⠙ ");
    assert_eq!(render_task_state_emoji("completed", 0), "✅");
    assert_eq!(render_task_state_emoji("failed", 0), "❌");
    assert_eq!(render_task_state_emoji("skipped", 0), "⏭ ");
}

#[test]
fn formats_durations_for_progress_rows() {
    assert_eq!(pretty_duration(999), "999ns");
    assert_eq!(pretty_duration(1_200), "1.2us");
    assert_eq!(pretty_duration(2_500_000), "2.5ms");
    assert_eq!(pretty_duration(2_000_000_000), "2.0s");
}
