use expect_test::expect;
use nao_base::error::NaoError;
use nao_base::logging::{info_span, init_logging};
use nao_base::result::NaoResult;

/* 📖 # Why keep error rendering tests in a separate file?
These assertions verify track_caller locations and rendered source lines, so the expected file and
line numbers are part of the behavior under test. Keeping them in a dedicated file avoids snapshot
churn when error.rs changes for unrelated implementation reasons.
*/
#[test]
fn err_macro_formats_error_with_caller_location() {
    let error = nao_base::err!("test {}", 123);

    expect!([r#"
        × error test 123
          at crates/base/tests/error_rendering.rs:13:17
    "#])
    .assert_eq(&error.to_test_string());
}

#[test]
fn bail_macro_formats_error_with_caller_location() {
    let error = (|| -> NaoResult<()> {
        nao_base::bail!("test {}", 123);
    })()
    .unwrap_err();

    expect!([r#"
        × error test 123
          at crates/base/tests/error_rendering.rs:25:9
    "#])
    .assert_eq(&error.to_test_string());
}

#[test]
fn chained_error_formats_cause_and_locations() {
    let error =
        NaoError::message("failed to read file").with_source(NaoError::message("missing file"));

    expect!([r#"
        × error failed to read file
          at crates/base/tests/error_rendering.rs:39:9
        caused by: missing file
             at crates/base/tests/error_rendering.rs:39:62
    "#])
    .assert_eq(&error.to_test_string());
}

#[test]
fn std_source_error_formats_cause_and_locations() {
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing config");
    let error = NaoError::message("cannot initialize").with_std_source(io_error);

    expect!([r#"
        × error cannot initialize
          at crates/base/tests/error_rendering.rs:53:17
        caused by: missing config
             at crates/base/tests/error_rendering.rs:53:56
    "#])
    .assert_eq(&error.to_test_string());
}

#[test]
fn span_trace_renders_as_structured_frames() {
    init_logging();
    let span = info_span!("error_test_span");
    let _guard = span.enter();

    let error = NaoError::message("failed inside span");

    expect!([r#"
        × error failed inside span
          at crates/base/tests/error_rendering.rs:70:17
          span trace:
            0: error_rendering::error_test_span
               at crates/base/tests/error_rendering.rs:67
    "#])
    .assert_eq(&error.to_test_string());
}

#[test]
fn chained_error_only_renders_root_cause_span_trace() {
    init_logging();

    let outer_span = info_span!("outer_error_span");
    let outer_guard = outer_span.enter();
    let error = {
        let inner_span = info_span!("inner_error_span");
        let _inner_guard = inner_span.enter();
        NaoError::message("outer failure").with_source(NaoError::message("root cause"))
    };
    drop(outer_guard);

    let rendered = error.to_test_string();

    assert!(rendered.contains("outer failure"));
    assert!(rendered.contains("caused by: root cause"));
    assert!(rendered.contains("inner_error_span"));
    assert!(rendered.contains("outer_error_span"));
    assert_eq!(rendered.matches("span trace:").count(), 1);
}
