#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use sweeper_contract::{
    RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::{StoredItem, TestScanSessionSeed};
use sweeper_desktop::AppState;
use sweeper_testkit::Sandbox;

pub struct SeedSpec {
    pub session_id: String,
    pub target_id: String,
    pub target_name: String,
    pub tier: TierDto,
    pub risk: RiskDto,
    pub requires_typed_confirmation: bool,
}

impl Default for SeedSpec {
    fn default() -> Self {
        Self {
            session_id: "scan-test".to_string(),
            target_id: "user-temp".to_string(),
            target_name: "User Temp".to_string(),
            tier: TierDto::Tier1,
            risk: RiskDto::Safe,
            requires_typed_confirmation: false,
        }
    }
}

pub struct SeedSessionInput {
    pub root: PathBuf,
    pub file_path: PathBuf,
    pub mtime_secs: u64,
    pub spec: SeedSpec,
}

pub fn seed_session(state: &AppState<FixedClock>, root: &Path, file_path: &Path, mtime_secs: u64) {
    seed_session_with_spec(
        state,
        SeedSessionInput {
            root: root.to_path_buf(),
            file_path: file_path.to_path_buf(),
            mtime_secs,
            spec: SeedSpec::default(),
        },
    );
}

pub fn seed_session_with_spec(state: &AppState<FixedClock>, input: SeedSessionInput) {
    let SeedSessionInput {
        root,
        file_path,
        mtime_secs,
        spec,
    } = input;
    let settings_gen = state.settings.lock().unwrap().generation();
    let size = fs::metadata(&file_path).map(|m| m.len()).unwrap_or(100);
    let item = ScanItemDto {
        id: "t1::1".to_string(),
        target_id: spec.target_id.clone(),
        path: file_path.to_string_lossy().to_string(),
        size: U64String(size),
        mtime_unix_secs: U64String(mtime_secs),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: spec.target_id.clone(),
        target_name: spec.target_name.clone(),
        tier: spec.tier,
        risk: spec.risk,
        resolved_paths: vec![root.to_string_lossy().to_string()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(size),
        eligible_bytes: U64String(size),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: spec.session_id.clone(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        StoredItem {
            dto: item.clone(),
            target_id: spec.target_id.clone(),
            target_name: spec.target_name.clone(),
            tier: tier_to_core(spec.tier),
            risk: risk_to_core(spec.risk),
            age_eligible: true,
            requires_typed_confirmation: spec.requires_typed_confirmation,
        },
    );
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: settings_gen,
            session_id: spec.session_id,
            results,
            items,
        });
}

pub fn tier_to_core(tier: TierDto) -> sweeper_core::Tier {
    match tier {
        TierDto::Tier1 => sweeper_core::Tier::Tier1,
        TierDto::Tier2 => sweeper_core::Tier::Tier2,
        TierDto::Tier3 => sweeper_core::Tier::Tier3,
    }
}

pub fn risk_to_core(risk: RiskDto) -> sweeper_core::Risk {
    match risk {
        RiskDto::Safe => sweeper_core::Risk::Safe,
        RiskDto::Caution => sweeper_core::Risk::Caution,
        RiskDto::Unknown => sweeper_core::Risk::Unknown,
        RiskDto::Protected => sweeper_core::Risk::Protected,
    }
}

/// A one-item seeded scan fixture. The sentinel sandbox and the seeded file
/// are exposed so tests can assert on deletion while the sandbox stays alive.
pub struct SeededScan {
    pub state: AppState<FixedClock>,
    pub sandbox: Sandbox,
    pub file: PathBuf,
}

/// Seed one scan item into a caller-supplied state and sandbox (for example a
/// state whose settings confine a real `start_scan` to the sentinel sandbox).
/// Returns the resolved seeded file path for deletion assertions.
pub fn seeded_scan_in(
    state: AppState<FixedClock>,
    sandbox: Sandbox,
    file_name: &str,
) -> SeededScan {
    let root = sandbox.create_dir("Temp").unwrap();
    let file = sandbox
        .write_file(format!("Temp\\{file_name}"), b"abc")
        .unwrap();
    seed_session(&state, &root, &file, 1_700_000_000);
    SeededScan {
        state,
        sandbox,
        file,
    }
}

pub fn seeded_scan() -> SeededScan {
    seeded_scan_in(crate::common::state::make_state(), Sandbox::new(), "a.tmp")
}

pub fn preview_scan_token(state: &AppState<FixedClock>) -> String {
    commands_impl::preview_clean(
        state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap()
    .preview_token
}
