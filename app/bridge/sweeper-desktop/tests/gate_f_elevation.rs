mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use sweeper_desktop::clean_gate::CleanGate;
use sweeper_desktop::commands_impl;
use sweeper_desktop::elevation::{
    relaunch_elevated as relaunch_pure, ElevationProvider, SingleInstanceState,
};

use common::FakeElevationProvider;

// --- Elevate graceful exit (#92) -------------------------------------------------
//
// These drive the pure relaunch seam directly so the ordering between an
// in-flight Clean settling, the elevated launch, and the old process exiting is
// deterministic. No real UAC prompt, process exit, or deletion is performed.
//
// Assertion helpers below are deliberately small (at most three assertions
// each) so the CodeScene "Large Assertion Blocks" gate stays satisfied while
// every behavioral check is preserved.

/// The old process, the elevated launch, and the guard handoff must all be held
/// back while a Clean is still in flight.
fn assert_shutdown_deferred(
    relaunch_calls: &AtomicUsize,
    exited: &AtomicBool,
    si: &SingleInstanceState,
) {
    assert_eq!(
        relaunch_calls.load(Ordering::SeqCst),
        0,
        "elevated launch must be deferred until the Clean settles"
    );
    assert!(
        !exited.load(Ordering::SeqCst),
        "process must not exit while a Clean is in flight"
    );
    assert!(
        si.is_held(),
        "single-instance guard must be held while waiting"
    );
}

/// The exit hook ran with the expected clean success code.
fn assert_exit_recorded(exited: &AtomicBool, exit_code: &Mutex<Option<i32>>) {
    assert!(
        exited.load(Ordering::SeqCst),
        "exit must follow Clean completion"
    );
    assert_eq!(*exit_code.lock().expect("exit code lock"), Some(0));
}

/// The elevated launch happened and the single-instance guard was handed off.
fn assert_handoff_complete(launched: bool, relaunch_calls: &AtomicUsize, si: &SingleInstanceState) {
    assert!(launched);
    assert_eq!(relaunch_calls.load(Ordering::SeqCst), 1);
    assert!(!si.is_held());
}

/// A sealed gate must reject any new Clean, both while an older Clean settles
/// and after a successful Elevate handoff (the old process is going away).
fn assert_clean_rejected(gate: &CleanGate, message: &str) {
    assert!(gate.enter().is_none(), "{message}");
}

/// Wait until the Elevate worker has actually sealed the gate, instead of
/// sleeping for a fixed duration that may end before the worker gets there.
///
/// `CleanGate` exposes no `is_sealed` accessor, so the observable seal signal is
/// `enter()` returning `None`. Each probe guard is dropped immediately: leaving
/// one alive would keep the worker blocked in `seal_and_wait_for_idle`, so the
/// caller's original Clean guard stays the only in-flight Clean throughout.
/// A hard deadline turns a stalled worker into a test failure rather than a hang.
fn wait_for_gate_sealed(gate: &CleanGate) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match gate.enter() {
            None => return,
            Some(probe) => drop(probe),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the Elevate worker to seal the Clean gate"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Elevation provider that parks inside `relaunch` until released, so a test
/// can hold one relaunch at the UAC-prompt stage while issuing a second call.
#[derive(Debug)]
struct BlockingProvider {
    outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>,
    release: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<AtomicBool>,
}

impl BlockingProvider {
    fn new(outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>) -> Self {
        Self {
            outcome,
            release: Arc::new((Mutex::new(false), Condvar::new())),
            entered: Arc::new(AtomicBool::new(false)),
        }
    }

    fn wait_until_blocked(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.entered.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the first relaunch to reach the UAC prompt"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn release(&self) {
        let (lock, cv) = self.release.as_ref();
        let mut done = lock.lock().expect("release lock");
        *done = true;
        cv.notify_all();
    }
}

impl ElevationProvider for BlockingProvider {
    fn is_elevated(&self) -> Result<bool, String> {
        Ok(false)
    }

    fn relaunch(&self) -> Result<sweeper_core::elevation::RelaunchOutcome, String> {
        self.entered.store(true, Ordering::SeqCst);
        let (lock, cv) = self.release.as_ref();
        let mut done = lock.lock().expect("release lock");
        while !*done {
            done = cv.wait(done).expect("release condvar");
        }
        self.outcome.clone()
    }
}

/// A concurrent relaunch is rejected with an internal error, never invokes its
/// launcher, and never unseals the gate owned by the first call.
fn assert_concurrent_rejected(
    err: &sweeper_contract::ContractError,
    contender: &FakeElevationProvider,
    gate: &CleanGate,
) {
    assert!(
        matches!(err, sweeper_contract::ContractError::Internal { .. }),
        "{err:?}"
    );
    assert_eq!(contender.relaunch_calls.load(Ordering::SeqCst), 0);
    assert!(
        gate.enter().is_none(),
        "a rejected concurrent relaunch must not unseal the gate"
    );
}

#[test]
fn relaunch_elevated_defers_launch_and_exit_until_in_flight_clean_settles() {
    let provider = Arc::new(FakeElevationProvider::new(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    ));
    let si = common::make_single_instance_state_with_unique_mutex("SweeperGateF-deferred-exit");
    let exited = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(Mutex::new(None));

    let exited_hook = exited.clone();
    let exit_code_hook = exit_code.clone();
    let exit_hook = move |code: i32| {
        exited_hook.store(true, Ordering::SeqCst);
        *exit_code_hook.lock().expect("exit code lock") = Some(code);
    };

    let gate = CleanGate::default();
    // Simulate a Clean currently in progress.
    let clean_guard = gate.enter().expect("gate starts unsealed");

    let provider_thread = provider.clone();
    let si_thread = Arc::clone(&si);
    let gate_thread = gate.clone();
    let relaunch = std::thread::spawn(move || {
        relaunch_pure(
            provider_thread.as_ref(),
            si_thread.as_ref(),
            &gate_thread,
            &exit_hook,
        )
    });

    // Keep the Clean guard held until the worker has sealed the gate, then
    // verify neither the elevated launch nor the exit happened in that window,
    // and that the single-instance guard still belongs to this instance.
    wait_for_gate_sealed(&gate);
    assert_shutdown_deferred(&provider.relaunch_calls, &exited, &si);

    // The Clean settles; the graceful path may now proceed.
    drop(clean_guard);
    let result = relaunch
        .join()
        .expect("relaunch thread panicked")
        .expect("relaunch ok");
    assert_exit_recorded(&exited, &exit_code);
    assert_handoff_complete(result.launched, &provider.relaunch_calls, &si);
}

#[test]
fn relaunch_elevated_never_exits_before_active_clean_completes() {
    let provider = Arc::new(FakeElevationProvider::new(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    ));
    let si = common::make_single_instance_state_with_unique_mutex("SweeperGateF-clean-ordering");
    let clean_finished = Arc::new(AtomicBool::new(false));
    let exit_saw_finished = Arc::new(AtomicBool::new(false));

    let finished_for_exit = clean_finished.clone();
    let saw_for_assert = exit_saw_finished.clone();
    let exit_hook = move |_code: i32| {
        saw_for_assert.store(finished_for_exit.load(Ordering::SeqCst), Ordering::SeqCst);
    };

    let gate = CleanGate::default();
    let clean_guard = gate.enter().expect("gate starts unsealed");
    let finished_for_clean = clean_finished.clone();
    let clean_thread = std::thread::spawn(move || {
        // A real Clean would run its safety-checked removal here. When it is
        // done it drops its guard, which is the only signal the exit waits on.
        std::thread::sleep(std::time::Duration::from_millis(40));
        finished_for_clean.store(true, Ordering::SeqCst);
        drop(clean_guard);
    });

    let provider_thread = provider.clone();
    let si_thread = Arc::clone(&si);
    let gate_thread = gate.clone();
    let relaunch = std::thread::spawn(move || {
        relaunch_pure(
            provider_thread.as_ref(),
            si_thread.as_ref(),
            &gate_thread,
            &exit_hook,
        )
    });

    clean_thread.join().expect("clean thread panicked");
    relaunch
        .join()
        .expect("relaunch thread panicked")
        .expect("relaunch ok");

    assert!(clean_finished.load(Ordering::SeqCst));
    assert!(
        exit_saw_finished.load(Ordering::SeqCst),
        "exit hook must observe the Clean as finished; an in-flight Clean must never be terminated"
    );
}

/// #92: sealing must close the window where a Clean could enter after the gate
/// was observed idle but before the old process handed off and exited.
#[test]
fn relaunch_elevated_seals_gate_and_rejects_new_clean_until_in_flight_clean_settles() {
    let ctx = common::make_elevation_test_state(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    );
    let gate = ctx.state.clean_gate.clone();
    // A Clean is already in flight when Elevate is requested.
    let clean_guard = gate.enter().expect("gate starts unsealed");

    let provider = ctx.provider.clone();
    let si = ctx.si.clone();
    let gate_for_relaunch = gate.clone();
    let exit_hook = ctx.state.app_exit.clone();
    let relaunch = std::thread::spawn(move || {
        relaunch_pure(
            provider.as_ref(),
            si.as_ref(),
            &gate_for_relaunch,
            exit_hook.as_ref(),
        )
    });

    // While the Clean is in flight the gate must be sealed: a new Clean is
    // rejected, and neither launch nor exit may happen. No Clean is terminated.
    // The original Clean guard is still held here, so observing the seal proves
    // the worker reached seal_and_wait_for_idle rather than simply not starting.
    wait_for_gate_sealed(&gate);
    assert_clean_rejected(
        &gate,
        "a new Clean must be rejected once the gate is sealed for shutdown",
    );
    assert_shutdown_deferred(&ctx.provider.relaunch_calls, &ctx.exited, &ctx.si);

    // The in-flight Clean settles; handoff and exit may now proceed.
    drop(clean_guard);
    let result = relaunch
        .join()
        .expect("relaunch thread panicked")
        .expect("relaunch ok");
    assert_handoff_complete(result.launched, &ctx.provider.relaunch_calls, &ctx.si);
    assert_exit_recorded(&ctx.exited, &ctx.exit_code);
    assert_clean_rejected(&gate, "gate must stay sealed after a successful relaunch");
}

/// #92: an aborted or failed relaunch must unseal the gate so the still-running
/// app can accept Cleans again.
#[test]
fn relaunch_elevated_abort_or_failure_unseals_gate() {
    fn run_aborted(outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>) {
        let ctx = common::make_elevation_test_state(false, outcome);
        let gate = ctx.state.clean_gate.clone();
        // Gate is open before the relaunch attempt.
        drop(gate.enter().expect("gate starts unsealed"));

        let err = commands_impl::relaunch_elevated(&ctx.state).unwrap_err();
        assert!(matches!(
            err,
            sweeper_contract::ContractError::InvalidInput { .. }
                | sweeper_contract::ContractError::Internal { .. }
        ));
        // App stays usable: a new Clean can enter after abort/failure.
        drop(
            gate.enter()
                .expect("gate must be unsealed after an aborted relaunch"),
        );
        assert!(!ctx.exited.load(Ordering::SeqCst));
        assert!(ctx.si.is_held());
    }

    run_aborted(Ok(
        sweeper_core::elevation::RelaunchOutcome::CancelledByUser,
    ));
    run_aborted(Err("ShellExecute failed".to_string()));
}

/// #92: a second concurrent relaunch must be rejected before it can seal or
/// unseal the gate. Without ownership tracking the second call would fail to
/// take the shared single-instance guard and unseal the gate while the first
/// call's UAC prompt is pending, letting a Clean start and later be killed at
/// exit — exactly the hazard this gate exists to prevent.
#[test]
fn relaunch_elevated_rejects_concurrent_relaunch_without_unsealing_gate() {
    let blocking = Arc::new(BlockingProvider::new(Ok(
        sweeper_core::elevation::RelaunchOutcome::Launched,
    )));
    let si = common::make_single_instance_state_with_unique_mutex("SweeperGateF-concurrent");
    let gate = CleanGate::default();
    let exited = Arc::new(AtomicBool::new(false));

    let exited_hook = exited.clone();
    let exit_hook = move |_code: i32| {
        exited_hook.store(true, Ordering::SeqCst);
    };
    let provider = blocking.clone();
    let si_thread = si.clone();
    let gate_thread = gate.clone();
    let first = std::thread::spawn(move || {
        relaunch_pure(
            provider.as_ref(),
            si_thread.as_ref(),
            &gate_thread,
            &exit_hook,
        )
    });

    // The first call now holds the guard with its UAC prompt pending and the
    // gate sealed.
    blocking.wait_until_blocked();
    assert_clean_rejected(&gate, "the gate must be sealed while UAC is pending");

    // The contender shares the gate and single-instance state: without the
    // ownership slot it would reach `take()`, fail, and unseal the gate.
    let contender = FakeElevationProvider::new(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    );
    let no_exit = |_code: i32| {};
    let err = relaunch_pure(&contender, si.as_ref(), &gate, &no_exit)
        .expect_err("a concurrent relaunch must be rejected");
    assert_concurrent_rejected(&err, &contender, &gate);

    blocking.release();
    let result = first
        .join()
        .expect("first relaunch thread panicked")
        .expect("first relaunch ok");
    assert!(result.launched);
    assert!(exited.load(Ordering::SeqCst));
    assert_clean_rejected(&gate, "the gate stays sealed after a successful handoff");
}
