use crate::error::NowError;
use crate::result::NowResult;
use std::fmt::Write as _;
use std::process::ExitCode;

/// What does `try_main` provide for CLI binaries?
///
/// It runs a fallible entrypoint, prints a readable error report on failure,
/// and converts the outcome into a process exit code.
pub fn try_main(run: impl FnOnce() -> NowResult<()>) -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprint!("{}", format_cli_error("operation failed", &error));
            ExitCode::FAILURE
        }
    }
}

/// What does `try_main_with_headline` provide?
///
/// It behaves like [`try_main`], but lets a binary choose a more specific
/// headline for its top-level error report.
pub fn try_main_with_headline(headline: &str, run: impl FnOnce() -> NowResult<()>) -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprint!("{}", format_cli_error(headline, &error));
            ExitCode::FAILURE
        }
    }
}

/// What does `format_cli_error` return?
///
/// It returns a stable, human-readable rendering of a [`NowError`] that is
/// suitable for printing from a command-line binary.
pub fn format_cli_error(headline: &str, error: &NowError) -> String {
    let mut rendered = String::new();
    let _ = writeln!(&mut rendered, "\u{1b}[1;31m━━ {}\u{1b}[0m", headline);
    error
        .write_to(&mut rendered)
        .expect("error rendering should not fail");

    let mut causes = Vec::new();
    let mut current = error.source();
    while let Some(cause) = current {
        causes.push((cause.kind().to_string(), cause.location()));
        current = cause.source();
    }

    if !causes.is_empty() {
        rendered.push('\n');
        rendered.push_str("\u{1b}[1;33m━━ cause chain\u{1b}[0m\n");
        for (cause, location) in &causes {
            let _ = writeln!(&mut rendered, "  • {}", cause);
            let _ = writeln!(
                &mut rendered,
                "    at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
    }

    rendered
}

#[cfg(test)]
mod tests {
    use crate::error::NowError;
    use expect_test::expect;

    use super::format_cli_error;

    #[test]
    fn format_cli_error_renders_headline_and_cause_chain() {
        let error = NowError::message("failed to verify")
            .with_source(NowError::message("missing reference output"));

        expect!([r#"
            ━━ verification failed
            × error failed to verify
              at crates/base/src/cli.rs:79:21
            caused by: missing reference output
                 at crates/base/src/cli.rs:80:26

            ━━ cause chain
              • missing reference output
                at crates/base/src/cli.rs:80:26
        "#])
        .assert_eq(&crate::unansi(&format_cli_error(
            "verification failed",
            &error,
        )));
    }
}
