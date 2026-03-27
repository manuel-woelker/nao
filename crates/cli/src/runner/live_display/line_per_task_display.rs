use crate::runner::live_display::shared::LiveTaskSnapshot;
use crate::runner::live_display::shared::LiveTaskStatus;
use crate::runner::live_display::shared::lock_mutex;
use crate::runner::live_display::shared::new_snapshot;
use crate::runner::live_display::shared::render_line_per_task_display;
use crate::runner::live_display::shared::store_async_error;
use crate::runner::live_display::shared::take_async_error;
use crate::runner::live_display::shared::write_stdout;
use nao_base::err;
use nao_base::result::NaoResult;
use nao_base::result::OptionExt;
use nao_base::result::ResultExt;
use nao_base::shared_string::SharedString;
use nao_engine::RunObserver;
use nao_recipe::Task;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

pub struct LinePerTaskDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl LinePerTaskDisplay {
    pub fn start(header: String, tasks: &[Task]) -> Self {
        let snapshot = Arc::new(Mutex::new(new_snapshot(header, tasks)));
        let stop = Arc::new(AtomicBool::new(false));
        let update_error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_snapshot = Arc::clone(&snapshot);
        let thread_update_error = Arc::clone(&update_error);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let line_count = lock_mutex(
                &thread_snapshot,
                "failed to lock line-per-task display snapshot",
            )?
            .tasks
            .len()
                + 1;
            let mut frame_index = 0usize;
            let mut first_render = true;

            loop {
                let rendered = {
                    let snapshot = lock_mutex(
                        &thread_snapshot,
                        "failed to lock line-per-task display snapshot",
                    )?;
                    render_line_per_task_display(snapshot.clone(), FRAMES[frame_index])
                };

                if first_render {
                    write_stdout(&rendered)
                        .context("failed to write initial line-per-task display frame")?;
                    first_render = false;
                } else {
                    write_stdout(&format!("\x1b[{line_count}F\x1b[J{rendered}"))
                        .context("failed to write line-per-task display frame")?;
                }

                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }

                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }

            if let Err(error) =
                write_stdout("").context("failed to flush line-per-task display output")
            {
                store_async_error(
                    &thread_update_error,
                    error,
                    "failed to persist line-per-task display error",
                );
            }
            Ok(())
        });

        Self {
            stop,
            snapshot,
            update_error,
            handle: Some(handle),
        }
    }

    pub fn finish(&mut self) -> NaoResult<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let result = handle
                .join()
                .map_err(|_| err!("line-per-task display thread panicked"))
                .context("failed to join line-per-task display thread")?;
            result?;
        }

        if let Some(error) = take_async_error(
            &self.update_error,
            "failed to collect line-per-task display error",
        )? {
            return Err(error);
        }

        Ok(())
    }

    fn update_task(&self, task_name: &str, status: LiveTaskStatus) {
        self.update_task_with_elapsed(task_name, status, None, None);
    }

    fn update_task_with_elapsed(
        &self,
        task_name: &str,
        status: LiveTaskStatus,
        elapsed_nanos: Option<u128>,
        outcome_message: Option<&str>,
    ) {
        let mut snapshot = match lock_mutex(
            &self.snapshot,
            "failed to lock line-per-task display snapshot",
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist line-per-task display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        let task = match snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
            .with_context(|| format!("task `{task_name}` is missing from line-per-task display"))
        {
            Ok(task) => task,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist line-per-task display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        task.status = status;
        task.elapsed_nanos = elapsed_nanos;
        task.outcome_message = outcome_message.map(SharedString::from);
    }
}

impl RunObserver for LinePerTaskDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Running);
    }

    fn on_task_completed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Completed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_failed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Failed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_skipped(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Skipped);
    }
}

impl Drop for LinePerTaskDisplay {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use crate::runner::live_display::shared::LiveTaskSnapshot;
    use crate::runner::live_display::shared::LiveTaskState;
    use crate::runner::live_display::shared::LiveTaskStatus;
    use crate::runner::live_display::shared::render_line_per_task_display;
    use expect_test::expect;
    use nao_base::shared_string::SharedString;

    #[test]
    fn renders_line_per_task_display() {
        let rendered = render_line_per_task_display(
            LiveTaskSnapshot {
                header: "Running test and 1 prerequisite task".to_owned(),
                tasks: vec![
                    LiveTaskState {
                        name: SharedString::from("build"),
                        status: LiveTaskStatus::Completed,
                        elapsed_nanos: Some(4_000_000),
                        outcome_message: Some(SharedString::from("build ready")),
                    },
                    LiveTaskState {
                        name: SharedString::from("test"),
                        status: LiveTaskStatus::Running,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("lint"),
                        status: LiveTaskStatus::Running,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("publish"),
                        status: LiveTaskStatus::Skipped,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("cleanup"),
                        status: LiveTaskStatus::Failed,
                        elapsed_nanos: Some(12_345_678_901),
                        outcome_message: Some(SharedString::from("3 files uploaded")),
                    },
                ],
            },
            "⠙",
        );

        expect![[r#"
            🚀 Running test and 1 prerequisite task
              ✅ build     0.004s  build ready     
              ⠙  test     running
              ⠙  lint     running
              ⏭  publish
              ❌ cleanup  12.346s  3 files uploaded
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }
}
