use crate::runner::live_display::write_stdout;
use crate::runner::rendering::pretty_duration;
use nao_base::result::NaoResult;
use nao_base::result::ResultExt;
use nao_engine::RunObserver;

#[derive(Default)]
pub struct CiDisplay {
    write_error: Option<nao_base::error::NaoError>,
}

impl CiDisplay {
    pub fn finish(&mut self) -> NaoResult<()> {
        match self.write_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn write_line(&mut self, line: &str, context: &str) {
        if self.write_error.is_some() {
            return;
        }

        if let Err(error) = write_stdout(&(line.to_owned() + "\n")).with_context(|| context) {
            self.write_error = Some(error);
        }
    }
}

impl RunObserver for CiDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.write_line(
            &format!("Starting {task_name}"),
            "failed to write CI task start",
        );
    }

    fn on_task_completed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        let outcome_suffix = outcome_message
            .map(|message| format!(": {message}"))
            .unwrap_or_default();
        self.write_line(
            &format!(
                "Completed {task_name} in {}{outcome_suffix}",
                pretty_duration(elapsed_nanos)
            ),
            "failed to write CI task completion",
        );
    }

    fn on_task_failed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        _outcome_message: Option<&str>,
    ) {
        self.write_line(
            &format!("Failed {task_name} in {}", pretty_duration(elapsed_nanos)),
            "failed to write CI task failure",
        );
    }
}
