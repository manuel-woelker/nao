use core::fmt::Write as _;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_engine::RunExecutionResult;
use nao_engine::RunStatus;
use nao_engine::RunTaskResult;
use nao_engine::TaskFailure;
use nao_recipe::TaskName;

pub fn render_success_summary(
    goal_tasks: &[SharedString],
    total_task_count: usize,
    duration_nanos: u128,
    goal_outcome_message: Option<&str>,
) -> String {
    let bold_goal_tasks = style_bold_white(&join_goal_tasks(goal_tasks));
    let bold_duration = style_bold_white(&pretty_duration(duration_nanos));
    let outcome_suffix = goal_outcome_message
        .map(|message| format!(": {}", style_bold_white(message)))
        .unwrap_or_default();

    format!(
        "✅ Succeeded {bold_goal_tasks} in {bold_duration} ({} {}){outcome_suffix}\n",
        total_task_count,
        if total_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
}

pub fn render_running_line(line_body: &str) -> String {
    format!("🚀 {line_body}\n")
}

pub fn render_failure_summary(goal_tasks: &[SharedString], task_failure: &TaskFailure) -> String {
    let mut output = String::new();
    let header_text = format!(
        "{} output: ({} lines omitted)",
        task_failure.task_name.as_str(),
        task_failure.omitted_output_line_count,
    );
    let side_width = 9usize;
    let side_line = "─".repeat(side_width);
    let _ = writeln!(
        &mut output,
        "╭{} {} {}╮",
        side_line,
        style_bold_white(&header_text),
        side_line,
    );

    for line in &task_failure.output_tail_lines {
        let _ = writeln!(&mut output, "{}", line.as_str());
    }

    let _ = writeln!(
        &mut output,
        "╰{} {} {}╯",
        side_line,
        style_bold_white(&header_text),
        side_line,
    );
    output.push('\n');

    let _ = writeln!(
        &mut output,
        "\u{1b}[1;31m❌\u{1b}[0m {} failed because {} failed with exit code {} in {} after {} completed successfully",
        style_bold_white(&join_goal_tasks(goal_tasks)),
        style_bold_white(task_failure.task_name.as_str()),
        style_bold_white(&task_failure.exit_code.to_string()),
        style_bold_white(&pretty_duration(task_failure.elapsed_nanos)),
        style_bold_white(&render_completed_task_count(
            task_failure.successful_task_count
        )),
    );

    output
}

pub fn render_ci_output(
    pal: &dyn nao_pal::pal::Pal,
    result: &RunExecutionResult,
) -> NaoResult<String> {
    let mut output = String::new();
    let mut executed_tasks = result
        .task_results
        .iter()
        .filter(|task| matches!(task.status.as_str(), "completed" | "failed"))
        .collect::<Vec<_>>();
    executed_tasks.sort_by_key(|task| (task.status.as_str() == "failed", task.name.as_str()));

    if !executed_tasks.is_empty() {
        output.push_str("Task logs\n");
        for (index, task) in executed_tasks.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            let _ = writeln!(
                &mut output,
                "== {} ({}) ==",
                task.name.as_str(),
                task.status.as_str()
            );
            output.push_str(&pal.read_file_to_string(&task.log_path)?);
        }
        output.push('\n');
    }

    output.push_str(&render_ci_summary(result));
    Ok(output)
}

pub fn render_running_line_body(goal_tasks: &[TaskName], total_task_count: usize) -> String {
    let prerequisite_task_count = total_task_count.saturating_sub(goal_tasks.len());
    format!(
        "Running {} and {} prerequisite {}",
        style_bold_white(&join_goal_task_names(goal_tasks)),
        prerequisite_task_count,
        if prerequisite_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
}

pub fn style_bold_white(text: &str) -> String {
    format!("\u{1b}[1;37m{text}\u{1b}[0m")
}

pub fn pretty_duration(duration_nanos: u128) -> String {
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

fn render_ci_summary(result: &RunExecutionResult) -> String {
    let mut output = String::new();
    let task_name_width = result
        .task_results
        .iter()
        .map(|task| task.name.as_str().len())
        .max()
        .unwrap_or(0);
    let status_width = result
        .task_results
        .iter()
        .map(|task| task.status.as_str().len())
        .max()
        .unwrap_or(0);
    let duration_width = result
        .task_results
        .iter()
        .filter_map(|task| task.duration_nanos)
        .map(pretty_duration)
        .map(|duration| duration.len())
        .max()
        .unwrap_or(1);

    let _ = writeln!(&mut output, "Run summary");
    for task in &result.task_results {
        let duration = task
            .duration_nanos
            .map(pretty_duration)
            .unwrap_or_else(|| "-".to_owned());
        let detail = render_ci_task_summary_detail(task);
        let _ = writeln!(
            &mut output,
            "  {:<status_width$}  {:<task_name_width$}  {:>duration_width$}{}",
            task.status.as_str(),
            task.name.as_str(),
            duration,
            detail
        );
    }

    let _ = writeln!(
        &mut output,
        "\nOverall result: {} in {}",
        match result.status {
            RunStatus::Completed => "completed",
            RunStatus::Failed(_) => "failed",
        },
        pretty_duration(result.duration_nanos)
    );
    if let RunStatus::Failed(task_failure) = &result.status {
        let _ = writeln!(
            &mut output,
            "Failure: {} failed with exit code {}",
            task_failure.task_name.as_str(),
            task_failure.exit_code
        );
    }

    output
}

fn render_ci_task_summary_detail(task: &RunTaskResult) -> String {
    let mut parts = Vec::new();
    if let Some(outcome_message) = &task.outcome_message {
        parts.push(outcome_message.as_str().to_owned());
    }
    if let Some(exit_code) = task.exit_code.filter(|exit_code| *exit_code != 0) {
        parts.push(format!("exit {exit_code}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("  {}", parts.join(" | "))
    }
}

fn join_goal_tasks(goal_tasks: &[SharedString]) -> String {
    goal_tasks
        .iter()
        .map(|task| task.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn join_goal_task_names(goal_tasks: &[TaskName]) -> String {
    goal_tasks
        .iter()
        .map(|task| task.as_str())
        .collect::<Vec<_>>()
        .join(",")
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

#[cfg(test)]
mod tests {
    use super::pretty_duration;
    use super::render_failure_summary;
    use super::render_running_line;
    use super::render_running_line_body;
    use super::render_success_summary;
    use expect_test::expect;
    use nao_base::shared_string::SharedString;
    use nao_engine::TaskFailure;
    use nao_recipe::TaskName;

    #[test]
    fn pretty_prints_durations() {
        expect!["4ns"].assert_eq(&pretty_duration(4));
        expect!["1.5us"].assert_eq(&pretty_duration(1_500));
        expect!["2.5ms"].assert_eq(&pretty_duration(2_500_000));
        expect!["3.0s"].assert_eq(&pretty_duration(3_000_000_000));
    }

    #[test]
    fn renders_multiple_goal_tasks_with_bold_comma_joining() {
        let rendered = render_success_summary(
            &[SharedString::from("lint"), SharedString::from("test")],
            5,
            2_500_000,
            None,
        );

        expect![[r#"
            ✅ Succeeded lint,test in 2.5ms (5 tasks)
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_running_line_with_prerequisite_count() {
        let rendered = render_running_line(&render_running_line_body(
            &[TaskName::from("lint"), TaskName::from("test")],
            5,
        ));

        expect![[r#"
            🚀 Running lint,test and 3 prerequisite tasks
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_success_summary_with_goal_outcome() {
        let rendered = render_success_summary(
            &[SharedString::from("test")],
            2,
            2_500_000,
            Some("30 tests succeeded"),
        );

        expect![[r#"
            ✅ Succeeded test in 2.5ms (2 tasks): 30 tests succeeded
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_failure_summary_for_multiple_completed_tasks() {
        let rendered = render_failure_summary(
            &[SharedString::from("fail5")],
            &TaskFailure {
                task_name: SharedString::from("fail3"),
                exit_code: 1,
                elapsed_nanos: 2_500_000,
                successful_task_count: 2,
                omitted_output_line_count: 0,
                output_tail_lines: vec![
                    SharedString::from("line one"),
                    SharedString::from("line two"),
                ],
            },
        );

        expect![[r#"
            ╭───────── fail3 output: (0 lines omitted) ─────────╮
            line one
            line two
            ╰───────── fail3 output: (0 lines omitted) ─────────╯

            ❌ fail5 failed because fail3 failed with exit code 1 in 2.5ms after 2 tasks completed successfully
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_failure_summary_with_omitted_line_notice() {
        let rendered = render_failure_summary(
            &[SharedString::from("fail5")],
            &TaskFailure {
                task_name: SharedString::from("fail3"),
                exit_code: 1,
                elapsed_nanos: 2_500_000,
                successful_task_count: 2,
                omitted_output_line_count: 23,
                output_tail_lines: vec![
                    SharedString::from("kept line 1"),
                    SharedString::from("kept line 2"),
                ],
            },
        );

        expect![[r#"
            ╭───────── fail3 output: (23 lines omitted) ─────────╮
            kept line 1
            kept line 2
            ╰───────── fail3 output: (23 lines omitted) ─────────╯

            ❌ fail5 failed because fail3 failed with exit code 1 in 2.5ms after 2 tasks completed successfully
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }
}
