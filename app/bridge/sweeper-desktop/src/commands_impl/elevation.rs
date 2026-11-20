use sweeper_contract::{ContractError, ElevationStatusDto, RelaunchResultDto};

use crate::clock::Clock;
use crate::AppState;

/// `get_elevation_status` implementation.
pub fn get_elevation_status<C: Clock>(
    state: &AppState<C>,
) -> Result<ElevationStatusDto, ContractError> {
    crate::elevation::get_elevation_status(state.elevation_provider.as_ref())
}

/// `relaunch_elevated` implementation.
pub fn relaunch_elevated<C: Clock>(
    state: &AppState<C>,
) -> Result<RelaunchResultDto, ContractError> {
    crate::elevation::relaunch_elevated(
        state.elevation_provider.as_ref(),
        state.single_instance.as_ref(),
        &state.clean_gate,
        state.app_exit.as_ref(),
    )
}
