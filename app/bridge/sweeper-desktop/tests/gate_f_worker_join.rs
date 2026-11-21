//! Gate F: a blocking worker's join error must be surfaced to callers, not
//! replaced by a constant reason that throws the failure away (#99).

use sweeper_contract::ContractError;
use sweeper_desktop::commands_impl::join_error_to_contract_error;

#[test]
fn blocking_worker_join_error_is_surfaced() {
    let err = join_error_to_contract_error("worker panicked while unlocking the session store");

    match err {
        ContractError::Internal { reason } => {
            assert!(
                reason.contains("worker panicked while unlocking the session store"),
                "join error detail was discarded: {reason}"
            );
        }
        other => panic!("expected a structured internal error, got {other:?}"),
    }
}

#[test]
fn arbitrary_worker_join_text_cannot_leak() {
    let secret = "database password hunter2 connection string";
    let err = join_error_to_contract_error(secret);

    match err {
        ContractError::Internal { reason } => {
            assert_eq!(
                reason, "background task failed",
                "arbitrary text leaked: {reason}"
            );
            assert!(!reason.contains(secret), "arbitrary text leaked: {reason}");
        }
        other => panic!("expected a structured internal error, got {other:?}"),
    }
}

#[test]
fn real_panicked_blocking_worker_maps_to_fixed_reason() {
    let handle = tauri::async_runtime::spawn_blocking(|| {
        panic!("synthetic panic payload SECRET_PANIC_TEXT");
    });
    let err: tauri::Error = tauri::async_runtime::block_on(async { handle.await.unwrap_err() });

    match join_error_to_contract_error(err) {
        ContractError::Internal { reason } => {
            assert_eq!(
                reason, "background task panicked",
                "unexpected reason: {reason}"
            );
            assert!(
                !reason.contains("SECRET_PANIC_TEXT"),
                "panic payload leaked: {reason}"
            );
            assert!(
                reason.chars().all(|c| !c.is_ascii_digit()),
                "task id leaked: {reason}"
            );
        }
        other => panic!("expected a structured internal error, got {other:?}"),
    }
}

#[test]
fn real_cancelled_task_maps_to_fixed_reason() {
    let handle = tauri::async_runtime::spawn(async {
        std::future::pending::<()>().await;
    });
    handle.abort();
    let err: tauri::Error = tauri::async_runtime::block_on(async { handle.await.unwrap_err() });

    assert_eq!(
        join_error_to_contract_error(err),
        ContractError::Internal {
            reason: "background task was cancelled".to_string()
        }
    );
}

#[test]
fn blocking_worker_join_error_stays_sanitized() {
    let err = join_error_to_contract_error("panic at C:\\Temp\\synthetic\\module.rs:42");

    match err {
        ContractError::Internal { reason } => {
            assert!(
                !reason.contains("C:\\Temp"),
                "internal path leaked: {reason}"
            );
            assert!(
                !reason.contains("module.rs"),
                "stack detail leaked: {reason}"
            );
        }
        other => panic!("expected a structured internal error, got {other:?}"),
    }
}
