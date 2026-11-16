use super::*;
use crate::{AppState, SystemClock};
use std::ffi::OsString;
use std::sync::Mutex;
use sweeper_contract::ContractError;

static E2E_READ_ONLY_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn execute_clean_rejects_when_e2e_read_only_is_enabled() {
    let _lock = E2E_READ_ONLY_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _guard = EnvVarGuard::set("SWEEPER_E2E_READ_ONLY", "1");
    let state = AppState::<SystemClock>::new_default();

    let err = execute_clean(&state, "preview-token".to_string(), None, |_| {}).unwrap_err();

    assert!(matches!(
        err,
        ContractError::Internal { reason } if reason == "Clean is disabled while SWEEPER_E2E_READ_ONLY=1 is active"
    ));
}

#[test]
fn execute_clean_preserves_normal_behavior_when_e2e_read_only_is_unset() {
    let _lock = E2E_READ_ONLY_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var_os("SWEEPER_E2E_READ_ONLY");
    std::env::remove_var("SWEEPER_E2E_READ_ONLY");
    let state = AppState::<SystemClock>::new_default();

    let err = execute_clean(&state, "no-such-token".to_string(), None, |_| {}).unwrap_err();

    match previous {
        Some(value) => std::env::set_var("SWEEPER_E2E_READ_ONLY", value),
        None => std::env::remove_var("SWEEPER_E2E_READ_ONLY"),
    }

    assert!(matches!(err, ContractError::PreviewTokenInvalid { .. }));
}

#[test]
fn sandbox_path_hint_preserves_real_tmp_paths() {
    let real = r"C:\Users\Public\AppData\Local\Temp\.tmpAbCd12\cache.bin";
    assert!(!is_sandbox_path_hint(real));
}

#[test]
fn sandbox_path_hint_matches_verified_sandbox_markers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sentinel = ".sweeper_sandbox_sentinel";
    std::fs::write(dir.path().join(sentinel), "").expect("write sentinel");
    let inside = dir.path().join("cache").join("a.tmp");
    assert!(is_sandbox_path_hint(&inside.to_string_lossy()));

    let by_component = dir.path().join(sentinel).join("cache");
    assert!(is_sandbox_path_hint(&by_component.to_string_lossy()));
}

#[test]
fn real_path_with_sweeper_test_substring_is_not_sandbox() {
    let path = r"C:\Users\Public\Documents\sweeper-test-results\report.json";
    assert!(!is_sandbox_path_hint(path));
}
