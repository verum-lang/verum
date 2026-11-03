use super::*;
#[test]
fn original_unicode_error_is_bounded_and_await_remains_lossless() {
    let mut queue = TaskQueue::new();
    queue.enable_diagnostics(1).unwrap();
    let id = queue.spawn(FunctionId(7));
    let message = "Ж".repeat(MAX_TASK_ERROR_BYTES);
    queue.fail_with(
        id,
        super::super::InterpreterError::Panic {
            message: message.clone(),
        },
    );
    let report = queue.diagnostics_report().unwrap();
    assert!(report.validate());
    let row = &report.rows[0];
    assert!(row.error_truncated);
    assert!(row.error.as_ref().unwrap().len() <= MAX_TASK_ERROR_BYTES);
    assert!(
        queue
            .get(id)
            .unwrap()
            .outcome()
            .unwrap_err()
            .to_string()
            .contains(&message)
    );
}
