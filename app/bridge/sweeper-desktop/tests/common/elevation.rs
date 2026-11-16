#![allow(dead_code)]

use std::sync::Arc;

use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::AppState;

pub fn make_single_instance_state_with_unique_mutex(
    prefix: &str,
) -> Arc<sweeper_desktop::elevation::SingleInstanceState> {
    static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mutex_name = format!(r"Local\{prefix}-{pid}-{nanos}-{n}");
    let guard = sweeper_core::elevation::SingleInstanceGuard::acquire_named(&mutex_name, 1_000)
        .expect("test should acquire single-instance mutex");
    Arc::new(sweeper_desktop::elevation::SingleInstanceState::with_guard(
        guard,
    ))
}

#[derive(Debug)]
pub struct FakeElevationProvider {
    elevated: bool,
    relaunch_outcome: std::sync::Mutex<Result<sweeper_core::elevation::RelaunchOutcome, String>>,
    pub relaunch_calls: std::sync::atomic::AtomicUsize,
    pub status_calls: std::sync::atomic::AtomicUsize,
}

impl FakeElevationProvider {
    pub fn new(
        elevated: bool,
        outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>,
    ) -> Self {
        Self {
            elevated,
            relaunch_outcome: std::sync::Mutex::new(outcome),
            relaunch_calls: std::sync::atomic::AtomicUsize::new(0),
            status_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl sweeper_desktop::elevation::ElevationProvider for FakeElevationProvider {
    fn is_elevated(&self) -> Result<bool, String> {
        self.status_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.elevated)
    }

    fn relaunch(&self) -> Result<sweeper_core::elevation::RelaunchOutcome, String> {
        self.relaunch_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.relaunch_outcome.lock().unwrap().clone()
    }
}

pub struct ElevationTestState {
    pub state: AppState<FixedClock>,
    pub provider: Arc<FakeElevationProvider>,
    pub exited: Arc<std::sync::atomic::AtomicBool>,
    pub exit_code: Arc<std::sync::Mutex<Option<i32>>>,
    pub si: Arc<sweeper_desktop::elevation::SingleInstanceState>,
}

pub fn make_elevation_test_state(
    elevated: bool,
    outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>,
) -> ElevationTestState {
    let provider = Arc::new(FakeElevationProvider::new(elevated, outcome));
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let exit_code = Arc::new(std::sync::Mutex::new(None));
    let exited_c = exited.clone();
    let exit_code_c = exit_code.clone();
    let exit_hook: Arc<dyn Fn(i32) + Send + Sync> = Arc::new(move |code| {
        exited_c.store(true, std::sync::atomic::Ordering::SeqCst);
        *exit_code_c.lock().unwrap() = Some(code);
    });
    let si = make_single_instance_state_with_unique_mutex("SweeperGateFElevation");
    let state = AppState::with_clock(FixedClock::new(1_800_000_000)).with_elevation_test_hooks(
        provider.clone(),
        si.clone(),
        exit_hook,
    );
    ElevationTestState {
        state,
        provider,
        exited,
        exit_code,
        si,
    }
}

pub fn assert_relaunch_cancel_preserved(
    ctx: &ElevationTestState,
    err: sweeper_contract::ContractError,
) {
    assert!(
        matches!(err, sweeper_contract::ContractError::InvalidInput { .. }),
        "{err:?}"
    );
    assert_eq!(
        ctx.provider
            .relaunch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(!ctx.exited.load(std::sync::atomic::Ordering::SeqCst));
    assert!(ctx.exit_code.lock().unwrap().is_none());
    assert!(ctx.si.is_held());
}
