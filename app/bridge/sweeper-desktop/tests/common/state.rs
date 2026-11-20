#![allow(dead_code)]

use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::AppState;

pub fn make_state() -> AppState<FixedClock> {
    AppState::with_clock(FixedClock::new(1_800_000_000))
}

pub fn state_with_temp_settings<F>(mutate: F) -> AppState<FixedClock>
where
    F: FnOnce(&mut sweeper_contract::SettingsDto),
{
    let state = make_state();
    let mut settings = state.settings.lock().unwrap().get();
    mutate(&mut settings);
    state
        .settings
        .lock()
        .unwrap()
        .update(settings)
        .expect("update settings in test helper");
    state
}
