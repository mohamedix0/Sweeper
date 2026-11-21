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
