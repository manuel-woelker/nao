use super::RunEngine;
use super::execution::extract_task_outcome_message;
#[cfg(not(windows))]
use super::process_command::build_bash_shell_script;
use super::process_command::build_process_command;
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
use nao_recipe::FailureMode;
use nao_recipe::LiveDisplay;
use nao_recipe::RunSpec;
use std::time::Duration;
use std::time::SystemTime;

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
fn builds_shell_tasks_with_strict_bash_flags() {
    let task = nao_recipe::Task {
        name: "test".into(),
        description: None,
        dependencies: Vec::new(),
        run: RunSpec::Shell(SharedString::from(
            "false\necho \"This should not be executed\"",
        )),
        environment: Vec::new(),
        artifacts: Vec::new(),
    };

    let command = build_process_command(&FilePath::from("nao.kdl"), &task).unwrap();

    #[cfg(not(windows))]
    assert_eq!(
        command,
        ProcessCommand {
            executable: SharedString::from("bash"),
            arguments: vec![
                SharedString::from("-o"),
                SharedString::from("errexit"),
                SharedString::from("-o"),
                SharedString::from("nounset"),
                SharedString::from("-o"),
                SharedString::from("errtrace"),
                SharedString::from("-o"),
                SharedString::from("pipefail"),
                SharedString::from("-c"),
                build_bash_shell_script("false\necho \"This should not be executed\""),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        }
    );

    #[cfg(windows)]
    assert_eq!(
        command,
        ProcessCommand {
            executable: SharedString::from("cmd"),
            arguments: vec![
                SharedString::from("/C"),
                SharedString::from("false\necho \"This should not be executed\""),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        }
    );
}

#[test]
fn builds_script_tasks_from_default_recipe_with_repository_root_working_directory() {
    let task = nao_recipe::Task {
        name: "check-code".into(),
        description: None,
        dependencies: Vec::new(),
        run: RunSpec::Script(FilePath::from("./scripts/check-code.sh")),
        environment: Vec::new(),
        artifacts: Vec::new(),
    };

    let command = build_process_command(&FilePath::from(".nao/nao.kdl"), &task).unwrap();

    assert_eq!(
        command,
        ProcessCommand {
            executable: SharedString::from("./scripts/check-code.sh"),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        }
    );
}

#[test]
fn builds_container_tasks_as_docker_run_commands() {
    let task = nao_recipe::Task {
        name: "image".into(),
        description: None,
        dependencies: Vec::new(),
        run: RunSpec::Container(nao_recipe::ContainerRunSpec {
            image: SharedString::from("alpine:3.22"),
            args: vec![
                SharedString::from("sh"),
                SharedString::from("-lc"),
                SharedString::from("printf hello"),
            ],
        }),
        environment: vec![nao_recipe::EnvironmentSpec {
            name: SharedString::from("RUST_LOG"),
            value: SharedString::from("warn"),
        }],
        artifacts: Vec::new(),
    };

    let command = build_process_command(&FilePath::from(".nao/nao.kdl"), &task).unwrap();

    assert_eq!(
        command,
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("--volume"),
                SharedString::from(".:/workspace"),
                SharedString::from("--workdir"),
                SharedString::from("/workspace"),
                SharedString::from("--env"),
                SharedString::from("RUST_LOG=warn"),
                SharedString::from("alpine:3.22"),
                SharedString::from("sh"),
                SharedString::from("-lc"),
                SharedString::from("printf hello"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        }
    );
}

#[test]
fn builds_compose_tasks_as_docker_compose_run_commands() {
    let task = nao_recipe::Task {
        name: "integration-test".into(),
        description: None,
        dependencies: Vec::new(),
        run: RunSpec::Compose(nao_recipe::ComposeRunSpec {
            directory: FilePath::from(".docker"),
            service: SharedString::from("rust"),
            args: vec![
                SharedString::from("bash"),
                SharedString::from("-lc"),
                SharedString::from("printf hello"),
            ],
        }),
        environment: vec![nao_recipe::EnvironmentSpec {
            name: SharedString::from("RUST_LOG"),
            value: SharedString::from("warn"),
        }],
        artifacts: Vec::new(),
    };

    let command = build_process_command(&FilePath::from(".nao/nao.kdl"), &task).unwrap();

    assert_eq!(
        command,
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("compose"),
                SharedString::from("-f"),
                SharedString::from(".docker/docker-compose.yaml"),
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("--env"),
                SharedString::from("RUST_LOG=warn"),
                SharedString::from("rust"),
                SharedString::from("bash"),
                SharedString::from("-lc"),
                SharedString::from("printf hello"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        }
    );
}

#[test]
#[cfg(not(windows))]
fn wraps_shell_tasks_with_err_trap_reporting() {
    let script = build_bash_shell_script("false\nprintf '%s\\n' \"$MISSING\"\ncat file | sort");

    expect![[r#"
trap 'rc=$?; printf "nao: command failed (exit %d) at line %d: %s\n" "$rc" "$LINENO" "$BASH_COMMAND" >&2; exit "$rc"' ERR
false
printf '%s\n' "$MISSING"
cat file | sort"#]]
    .assert_eq(script.as_str());
}

#[test]
fn extracts_last_task_outcome_message() {
    let outcome = extract_task_outcome_message(&[
        (
            Timestamp::new(1),
            ProcessOutputStream::Stdout,
            "Task outcome: discovering files".to_owned(),
        ),
        (
            Timestamp::new(2),
            ProcessOutputStream::Stdout,
            "ordinary output".to_owned(),
        ),
        (
            Timestamp::new(3),
            ProcessOutputStream::Stdout,
            "Task outcome: 30 tests succeeded".to_owned(),
        ),
    ]);

    assert_eq!(outcome.as_deref(), Some("30 tests succeeded"));
}

#[test]
fn executes_container_tasks_with_generated_docker_command() {
    let pal = PalMock::new();
    pal.set_file(
        ".nao/nao.kdl",
        r#"
            recipe "default" {
              task "image" {
                env RUST_LOG="warn"
                run container="alpine:3.22" {
                  args "sh" "-lc" "printf 'Task outcome: packaged\n'"
                }
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("--volume"),
                SharedString::from(".:/workspace"),
                SharedString::from("--workdir"),
                SharedString::from("/workspace"),
                SharedString::from("--env"),
                SharedString::from("RUST_LOG=warn"),
                SharedString::from("alpine:3.22"),
                SharedString::from("sh"),
                SharedString::from("-lc"),
                SharedString::from("printf 'Task outcome: packaged\n'"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(1),
                stream: ProcessOutputStream::Stdout,
                bytes: b"Task outcome: packaged\n".to_vec(),
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(3),
                stream: ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(4),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(4),
            exit_code: Some(0),
        },
    );

    let result = RunEngine::new(PalHandle::new(pal))
        .execute_run(&FilePath::from(".nao/nao.kdl"), &["image".to_owned()])
        .unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.total_task_count, 1);
    assert_eq!(
        result.task_results[0].outcome_message.as_deref(),
        Some("packaged")
    );
}

#[test]
fn preserves_output_for_failed_container_tasks() {
    let pal = PalMock::new();
    pal.set_file(
        ".nao/nao.kdl",
        r#"
            recipe "default" {
              task "image" {
                run container="alpine:3.22" {
                  args "sh" "-lc" "printf 'image build failed\n' >&2; exit 5"
                }
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("--volume"),
                SharedString::from(".:/workspace"),
                SharedString::from("--workdir"),
                SharedString::from("/workspace"),
                SharedString::from("alpine:3.22"),
                SharedString::from("sh"),
                SharedString::from("-lc"),
                SharedString::from("printf 'image build failed\n' >&2; exit 5"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(1),
                stream: ProcessOutputStream::Stderr,
                bytes: b"image build failed\n".to_vec(),
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(3),
                stream: ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(4),
                exit_code: Some(5),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(4),
            exit_code: Some(5),
        },
    );

    let result = RunEngine::new(PalHandle::new(pal))
        .execute_run(&FilePath::from(".nao/nao.kdl"), &["image".to_owned()])
        .unwrap();

    assert_eq!(
        result.status,
        RunStatus::Failed(TaskFailure {
            task_name: SharedString::from("image"),
            exit_code: 5,
            elapsed_nanos: 4,
            successful_task_count: 0,
            omitted_output_line_count: 0,
            output_tail_lines: vec![SharedString::from("image build failed")],
        })
    );
    assert!(result.output.contains("image build failed"));
}

#[test]
fn executes_compose_tasks_with_generated_docker_compose_command() {
    let pal = PalMock::new();
    pal.set_file(
        ".nao/nao.kdl",
        r#"
            recipe "default" {
              task "compose-hello" {
                env RUST_LOG="warn"
                run compose=".docker" service="rust" {
                  args "bash" "-lc" "printf 'Task outcome: greeted from compose\n'"
                }
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("compose"),
                SharedString::from("-f"),
                SharedString::from(".docker/docker-compose.yaml"),
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("--env"),
                SharedString::from("RUST_LOG=warn"),
                SharedString::from("rust"),
                SharedString::from("bash"),
                SharedString::from("-lc"),
                SharedString::from("printf 'Task outcome: greeted from compose\n'"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(1),
                stream: ProcessOutputStream::Stdout,
                bytes: b"Task outcome: greeted from compose\n".to_vec(),
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(3),
                stream: ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(4),
                exit_code: Some(0),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(4),
            exit_code: Some(0),
        },
    );

    let result = RunEngine::new(PalHandle::new(pal))
        .execute_run(
            &FilePath::from(".nao/nao.kdl"),
            &["compose-hello".to_owned()],
        )
        .unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(
        result.task_results[0].outcome_message.as_deref(),
        Some("greeted from compose")
    );
}

#[test]
fn preserves_output_for_failed_compose_tasks() {
    let pal = PalMock::new();
    pal.set_file(
        ".nao/nao.kdl",
        r#"
            recipe "default" {
              task "compose-hello" {
                run compose=".docker" service="rust" {
                  args "bash" "-lc" "printf 'compose failed\n' >&2; exit 7"
                }
              }
            }
            "#,
    );
    pal.set_process_execution(
        ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: vec![
                SharedString::from("compose"),
                SharedString::from("-f"),
                SharedString::from(".docker/docker-compose.yaml"),
                SharedString::from("run"),
                SharedString::from("--rm"),
                SharedString::from("rust"),
                SharedString::from("bash"),
                SharedString::from("-lc"),
                SharedString::from("printf 'compose failed\n' >&2; exit 7"),
            ],
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(1),
                stream: ProcessOutputStream::Stderr,
                bytes: b"compose failed\n".to_vec(),
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(2),
                stream: ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(3),
                stream: ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(4),
                exit_code: Some(7),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(0),
            finished_at: Timestamp::new(4),
            exit_code: Some(7),
        },
    );

    let result = RunEngine::new(PalHandle::new(pal))
        .execute_run(
            &FilePath::from(".nao/nao.kdl"),
            &["compose-hello".to_owned()],
        )
        .unwrap();

    assert_eq!(
        result.status,
        RunStatus::Failed(TaskFailure {
            task_name: SharedString::from("compose-hello"),
            exit_code: 7,
            elapsed_nanos: 4,
            successful_task_count: 0,
            omitted_output_line_count: 0,
            output_tail_lines: vec![SharedString::from("compose failed")],
        })
    );
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
    assert_eq!(plan.failure_mode, FailureMode::FailEarly);
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
fn plans_requested_failure_mode() {
    let pal = PalMock::new();
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              config failure-mode="fail-late"

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

    assert_eq!(plan.failure_mode, FailureMode::FailLate);
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

    let rendered = error.to_test_string();
    assert!(rendered.contains("task specifier `slow_` did not match any tasks"));
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
    assert_eq!(output.goal_outcome_message, None);
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
WRITE FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"requested_tasks":["test"],"timestamp":"1970-01-01T00:00:00Z","type":"run_started"}

APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"task":"build","timestamp":"1970-01-01T00:00:00Z","type":"task_started"}

RUN PROCESS: ./scripts/build.sh 
APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/build.log -> [1970-01-01T00:00:00Z] stdout: building

APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"duration_nanos":"4","exit_code":0,"outcome_message":null,"result":"success","status":"completed","task":"build","timestamp":"1970-01-01T00:00:00Z","type":"task_finished"}

APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_started"}

RUN PROCESS: ./scripts/test.sh 
APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/test.log -> [1970-01-01T00:00:00Z] stdout: testing

APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"duration_nanos":"4","exit_code":0,"outcome_message":null,"result":"success","status":"completed","task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_finished"}

APPEND FILE: .nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl -> {"result":"completed","timestamp":"1970-01-01T00:00:00Z","type":"run_finished"}

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
      "outcome_message": null,
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
      "outcome_message": null,
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
    assert!(summary.contains(
        "\"failure_message\": \"task `test` failed with exit code 1 after 4ns (1 task completed successfully)\""
    ));
}

#[test]
fn reports_failed_task_duration_relative_to_task_start() {
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
    pal.set_process_execution(
        ProcessCommand {
            executable: SharedString::from("./scripts/test.sh"),
            arguments: Vec::new(),
            working_directory: Some(FilePath::from(".")),
            environment: Vec::new(),
        },
        vec![
            ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(11),
                stream: ProcessOutputStream::Stdout,
                bytes: b"boom\n".to_vec(),
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(12),
                stream: ProcessOutputStream::Stdout,
            }),
            ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(13),
                stream: ProcessOutputStream::Stderr,
            }),
            ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(14),
                exit_code: Some(1),
            }),
        ],
        ProcessResult {
            started_at: Timestamp::new(10),
            finished_at: Timestamp::new(14),
            exit_code: Some(1),
        },
    );
    let engine = RunEngine::new(PalHandle::new(pal));

    let result = engine
        .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
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
}

#[test]
fn captures_outcome_from_direct_output_and_keeps_marker_in_logs() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
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
    set_script_process(
        &pal,
        "./scripts/test.sh",
        &[b"Task outcome: 30 tests succeeded\n", b"done\n"],
        0,
    );
    let engine = RunEngine::new(PalHandle::new(pal.clone()));

    let result = engine
        .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
        .unwrap();

    assert_eq!(
        result.goal_outcome_message,
        Some(SharedString::from("30 tests succeeded"))
    );
    let summary = pal
        .read_file_string(".nao/runs/1970-01-01T00-00-00Z-test/nao-summary.json")
        .unwrap();
    let log = pal
        .read_file_string(".nao/runs/1970-01-01T00-00-00Z-test/test.log")
        .unwrap();
    let events = pal
        .read_file_string(".nao/runs/1970-01-01T00-00-00Z-test/nao-events.jsonl")
        .unwrap();

    assert!(summary.contains("\"outcome_message\": \"30 tests succeeded\""));
    assert!(events.contains("\"outcome_message\":\"30 tests succeeded\""));
    assert!(log.contains("Task outcome: 30 tests succeeded"));
}

#[test]
fn keeps_last_directly_reported_outcome_message() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
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
    set_script_process(
        &pal,
        "./scripts/test.sh",
        &[
            b"Task outcome: starting\n",
            b"Task outcome: 12 files formatted\n",
        ],
        0,
    );
    let engine = RunEngine::new(PalHandle::new(pal.clone()));

    let result = engine
        .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
        .unwrap();

    assert_eq!(
        result.goal_outcome_message,
        Some(SharedString::from("12 files formatted"))
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

#[test]
fn fail_late_continues_unrelated_tasks_and_skips_only_blocked_dependents() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_file(
        "nao.kdl",
        r#"
            recipe "default" {
              config failure-mode="fail-late" max-parallel-tasks=2

              task "lint" {
                run script="./scripts/lint.sh"
              }

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
    set_script_process(&pal, "./scripts/lint.sh", &[b"lint ok\n"], 0);
    set_script_process(&pal, "./scripts/build.sh", &[b"boom\n"], 1);
    set_script_process(&pal, "./scripts/test.sh", &[b"should not run\n"], 0);
    set_script_process(&pal, "./scripts/package.sh", &[b"should not run\n"], 0);
    let engine = RunEngine::new(PalHandle::new(pal.clone()));

    let result = engine
        .execute_run(&FilePath::from("nao.kdl"), &["lint,package".to_owned()])
        .unwrap();

    assert!(matches!(result.status, RunStatus::Failed(_)));
    let summary = pal
        .read_file_string(".nao/runs/1970-01-01T00-00-00Z-lint+package/nao-summary.json")
        .unwrap();
    let events = pal
        .read_file_string(".nao/runs/1970-01-01T00-00-00Z-lint+package/nao-events.jsonl")
        .unwrap();

    assert!(summary.contains("\"name\": \"lint\""));
    assert!(summary.contains("\"status\": \"completed\""));
    assert!(summary.contains("\"name\": \"build\""));
    assert!(summary.contains("\"status\": \"failed\""));
    assert!(summary.contains("\"name\": \"test\""));
    assert!(summary.contains("\"status\": \"skipped\""));
    assert!(summary.contains("\"name\": \"package\""));
    assert!(summary.contains("\"status\": \"skipped\""));
    assert!(
        events.lines().any(|line| line.contains("\"task\":\"lint\"")
            && line.contains("\"type\":\"task_finished\""))
    );
    assert!(!events.lines().any(
        |line| line.contains("\"task\":\"test\"") && line.contains("\"type\":\"task_started\"")
    ));
    assert!(
        !events
            .lines()
            .any(|line| line.contains("\"task\":\"package\"")
                && line.contains("\"type\":\"task_started\""))
    );
}

#[test]
fn retries_run_directory_reservation_on_collision() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    pal.set_directory(".nao/runs/1970-01-01T00-00-00Z-test");
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
    set_script_process(&pal, "./scripts/test.sh", &[b"ok\n"], 0);
    let engine = RunEngine::new(PalHandle::new(pal.clone()));

    let result = engine
        .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
        .unwrap();

    assert_eq!(
        result.run_directory,
        FilePath::from(".nao/runs/1970-01-01T00-00-01Z-test")
    );
    pal.verify_effects(expect![
        r#"READ FILE: nao.kdl
CREATE DIRECTORY: .nao/runs
CREATE DIRECTORY: .nao/runs/1970-01-01T00-00-00Z-test
SLEEP: 1000ms
CREATE DIRECTORY: .nao/runs/1970-01-01T00-00-01Z-test
WRITE FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-plan.json -> {
  "requested_tasks": [
    "test"
  ],
  "tasks": [
    {
      "artifacts": [],
      "dependencies": [],
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
WRITE FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-events.jsonl -> {"requested_tasks":["test"],"timestamp":"1970-01-01T00:00:00Z","type":"run_started"}

APPEND FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-events.jsonl -> {"task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_started"}

RUN PROCESS: ./scripts/test.sh 
APPEND FILE: .nao/runs/1970-01-01T00-00-01Z-test/test.log -> [1970-01-01T00:00:00Z] stdout: ok

APPEND FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-events.jsonl -> {"duration_nanos":"4","exit_code":0,"outcome_message":null,"result":"success","status":"completed","task":"test","timestamp":"1970-01-01T00:00:00Z","type":"task_finished"}

APPEND FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-events.jsonl -> {"result":"completed","timestamp":"1970-01-01T00:00:00Z","type":"run_finished"}

WRITE FILE: .nao/runs/1970-01-01T00-00-01Z-test/nao-summary.json -> {
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
      "log_file": "test.log",
      "name": "test",
      "outcome_message": null,
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
fn errors_after_thirty_run_directory_collisions() {
    let pal = PalMock::new();
    pal.set_current_system_time(SystemTime::UNIX_EPOCH);
    for second in 0..30 {
        pal.set_directory(&format!(".nao/runs/1970-01-01T00-00-{second:02}Z-test"));
    }
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

    let error = engine
        .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
        .unwrap_err();

    assert!(
        error
            .to_test_string()
            .contains("Unable to reserve a unique run directory after 30 attempts")
    );
}
