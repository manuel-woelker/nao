use crate::runner::live_display::shared::LiveTaskSnapshot;
use crate::runner::live_display::shared::LiveTaskStatus;
use crate::runner::live_display::shared::lock_mutex;
use crate::runner::live_display::shared::new_snapshot;
use crate::runner::live_display::shared::render_single_line_display;
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

pub struct SingleLineDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl SingleLineDisplay {
    pub fn start(header: String, tasks: &[Task]) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(new_snapshot(header, tasks)));
        let update_error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_snapshot = Arc::clone(&snapshot);
        let thread_update_error = Arc::clone(&update_error);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut frame_index = 0usize;

            while !thread_stop.load(Ordering::Relaxed) {
                let rendered = {
                    let snapshot = lock_mutex(
                        &thread_snapshot,
                        "failed to lock single-line display snapshot",
                    )?;
                    render_single_line_display(snapshot.clone())
                };
                write_stdout(&format!("\r{} {}\x1b[K", FRAMES[frame_index], rendered))
                    .context("failed to write single-line display frame")?;
                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }

            let rendered = {
                let snapshot = lock_mutex(
                    &thread_snapshot,
                    "failed to lock single-line display snapshot",
                )?;
                render_single_line_display(snapshot.clone())
            };
            let write_result = write_stdout(&format!("\r🚀 {}\x1b[K\n", rendered))
                .context("failed to write final single-line display frame");
            if let Err(error) = write_result {
                store_async_error(
                    &thread_update_error,
                    error,
                    "failed to persist single-line display error",
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
                .map_err(|_| err!("single-line display thread panicked"))
                .context("failed to join single-line display thread")?;
            result?;
        }

        if let Some(error) = take_async_error(
            &self.update_error,
            "failed to collect single-line display error",
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
            "failed to lock single-line display snapshot",
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist single-line display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        let task = match snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
            .with_context(|| format!("task `{task_name}` is missing from single-line display"))
        {
            Ok(task) => task,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist single-line display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        task.status = status;
        task.elapsed_nanos = elapsed_nanos;
        task.status_message = None;
        task.outcome_message = outcome_message.map(SharedString::from);
    }
}

impl RunObserver for SingleLineDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Running);
    }

    fn on_task_status(&mut self, task_name: &str, status_message: &str) {
        let mut snapshot = match lock_mutex(
            &self.snapshot,
            "failed to lock single-line display snapshot",
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist single-line display update error",
                );
                return;
            }
        };
        if let Some(task) = snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
        {
            task.status_message = Some(SharedString::from(status_message));
        }
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

impl Drop for SingleLineDisplay {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use crate::runner::live_display::shared::LiveTaskSnapshot;
    use crate::runner::live_display::shared::LiveTaskState;
    use crate::runner::live_display::shared::LiveTaskStatus;
    use crate::runner::live_display::shared::render_single_line_display;
    use expect_test::expect;
    use nao_base::shared_string::SharedString;

    #[test]
    fn renders_single_line_display_with_concurrent_progress() {
        let rendered = render_single_line_display(LiveTaskSnapshot {
            header: "Running test and 2 prerequisite tasks".to_owned(),
            tasks: vec![
                LiveTaskState {
                    name: SharedString::from("build"),
                    status: LiveTaskStatus::Completed,
                    elapsed_nanos: Some(4_000_000),
                    status_message: None,
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("lint"),
                    status: LiveTaskStatus::Running,
                    elapsed_nanos: None,
                    status_message: Some(SharedString::from("2/5")),
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("test"),
                    status: LiveTaskStatus::Running,
                    elapsed_nanos: None,
                    status_message: None,
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("publish"),
                    status: LiveTaskStatus::Pending,
                    elapsed_nanos: None,
                    status_message: None,
                    outcome_message: None,
                },
            ],
        });

        expect!["Running test and 2 prerequisite tasks (running: 2, completed: 1, remaining: 1) — lint: 2/5"]
            .assert_eq(&nao_base::unansi(&rendered));
    }
}
