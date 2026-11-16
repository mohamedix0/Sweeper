#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sweeper_contract::{
    AnalyzerNodeDto, ContractError, ExecuteCleanResponse, PreviewCleanResponse,
    PreviewConfirmationsDto, RiskDto, ScanItemDto, ScanProgressEvent, ScanResultsDto,
    TargetScanResultDto, TierDto, U64String,
};
use sweeper_core::analyzer::{AnalyzerNode, AnalyzerProgress, AnalyzerResult};
use sweeper_core::Risk;
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::{AnalyzerSessionInput, StoredItem, TestScanSessionSeed};
use sweeper_desktop::AppState;
use sweeper_testkit::Sandbox;

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
    state.settings.lock().unwrap().update(settings);
    state
}

pub fn sandbox_test_base() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target-fallback"))
        .join("sweeper-test-sandboxes");
    let _ = fs::create_dir_all(&base);
    match fs::canonicalize(&base) {
        Ok(c) => normalize_canonical_path(&c),
        Err(_) => base.components().collect::<PathBuf>(),
    }
}

fn normalize_canonical_path(c: &Path) -> PathBuf {
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    c.to_path_buf()
}

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

pub struct SeededScan {
    pub state: AppState<FixedClock>,
    pub _sandbox: Sandbox,
}

pub fn seeded_scan() -> SeededScan {
    let state = make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    SeededScan {
        state,
        _sandbox: sandbox,
    }
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

pub fn read_tauri_json(file: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = match file {
        "cap" => root.join("capabilities").join("default.json"),
        _ => root.join(file),
    };
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

pub fn assert_tauri_csp_and_security(conf: &serde_json::Value) {
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");
}

pub fn assert_tauri_no_fs_shell(conf: &serde_json::Value, cap: &serde_json::Value) {
    let conf_s = conf.to_string().to_lowercase();
    let cap_s = cap.to_string().to_lowercase();
    for (hay, needle, label) in [
        (&conf_s, "plugin-fs", "config plugins"),
        (&conf_s, "plugin-shell", "config plugins"),
        (&cap_s, "fs:", "capability plugins"),
        (&cap_s, "shell:", "capability plugins"),
    ] {
        assert!(!hay.contains(needle), "{label}");
    }
}

pub fn assert_tauri_bundle_snapshot(conf: &serde_json::Value) {
    assert_eq!(conf["bundle"]["active"], serde_json::json!(true));
    let targets = conf["bundle"]["targets"].as_array().unwrap();
    let has_nsis = targets.iter().any(|t| t.as_str() == Some("nsis"));
    assert!(has_nsis, "{targets:?}");
    let icons = conf["bundle"]["icon"].as_array().unwrap();
    assert!(!icons.is_empty(), "bundle icons must not be empty");
}

pub fn find_node_by_path_suffix<'a>(
    node: &'a AnalyzerNodeDto,
    suffix: &str,
) -> Option<&'a AnalyzerNodeDto> {
    if node
        .path
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|c| find_node_by_path_suffix(c, suffix))
}

pub fn insert_analyzer_session(
    state: &AppState<FixedClock>,
    session_id: &str,
    root: AnalyzerNode,
    min_age_hours: u32,
) {
    let settings_gen = state.settings.lock().unwrap().generation();
    state
        .sessions
        .lock()
        .unwrap()
        .insert_test_analyzer_session(AnalyzerSessionInput {
            id: session_id.to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: AnalyzerResult {
                root,
                progress: AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours,
        });
}

pub fn assert_unknown_file_not_selectable(root_node: &AnalyzerNodeDto) {
    let file_node =
        find_node_by_path_suffix(root_node, "\\StaleCache\\old.txt").expect("old.txt node");
    assert!(!file_node.selectable, "reason={:?}", file_node.reason);
    assert_eq!(
        file_node.reason.as_deref(),
        Some("Unknown files are not selectable; only eligible directories can be selected"),
        "{:?}",
        file_node.reason
    );
}

pub fn assert_stale_cache_dir_selectable(root_node: &AnalyzerNodeDto) -> AnalyzerNodeDto {
    let node = find_node_by_path_suffix(root_node, "\\StaleCache").expect("StaleCache node");
    assert!(node.is_dir, "StaleCache must be a directory node");
    assert!(node.selectable, "reason={:?}", node.reason);
    assert!(!node.display_only, "StaleCache must not be display_only");
    let treated_as_custom = node.target_id.is_none() || node.risk == RiskDto::Unknown;
    assert!(treated_as_custom, "{node:?}");
    node.clone()
}

pub fn exec_analyzer_delete_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    node_id: String,
) -> ExecuteCleanResponse {
    let preview = preview_with_delete_confirmation(state, session_id, node_id).unwrap();
    assert!(preview.total_selected_count.0 >= 1);
    let logs_base = tempfile::tempdir().unwrap();
    execute_preview_token(state, preview.preview_token, logs_base.path())
}

pub struct AnalyzerNodeSeed<'a> {
    pub id: &'a str,
    pub path: &'a str,
    pub name: &'a str,
    pub is_dir: bool,
}

pub fn base_analyzer_node(seed: AnalyzerNodeSeed<'_>) -> AnalyzerNode {
    AnalyzerNode {
        id: seed.id.to_string(),
        path: seed.path.to_string(),
        name: seed.name.to_string(),
        is_dir: seed.is_dir,
        size_bytes: 0,
        item_count: 0,
        risk: Risk::Safe,
        description: String::new(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: false,
        selection_reason: None,
        children: Vec::new(),
    }
}

pub fn preview_with_delete_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    item_id: String,
) -> Result<PreviewCleanResponse, sweeper_contract::ContractError> {
    commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![item_id],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
}

pub fn execute_preview_token(
    state: &AppState<FixedClock>,
    preview_token: String,
    logs_base: &Path,
) -> ExecuteCleanResponse {
    commands_impl::execute_clean(state, preview_token, Some(logs_base.to_path_buf()), |_| {})
        .unwrap()
}

pub fn assert_session_invalid_err(err: ContractError) {
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

pub fn assert_preview_token_invalid_err(err: ContractError) {
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

pub fn assert_invalid_input_err(err: ContractError) {
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}

pub fn assert_session_or_preview_invalid(err: ContractError) {
    let is_session = matches!(err, ContractError::SessionInvalid { .. });
    let is_preview = matches!(err, ContractError::PreviewTokenInvalid { .. });
    assert!(is_session || is_preview, "{err:?}");
}

pub fn assert_unknown_or_invalid(err: ContractError) {
    let is_unknown = matches!(err, ContractError::UnknownItem { .. });
    let is_invalid = matches!(err, ContractError::InvalidInput { .. });
    assert!(is_unknown || is_invalid, "{err:?}");
}

pub fn assert_scan_progress_events(events: &[ScanProgressEvent], session_id: &str) {
    assert!(!events.is_empty(), "expected scan progress events");
    assert_scan_started(&events[0], session_id);
    assert_scan_has_progress(events);
    assert_scan_completed(events.last().unwrap(), session_id);
}

fn assert_scan_started(event: &ScanProgressEvent, session_id: &str) {
    match event {
        ScanProgressEvent::Started { session_id: id } => assert_eq!(id, session_id),
        other => panic!("expected Started as first event, got {other:?}"),
    }
}

fn assert_scan_completed(event: &ScanProgressEvent, session_id: &str) {
    match event {
        ScanProgressEvent::Completed { session_id: id } => assert_eq!(id, session_id),
        other => panic!("expected Completed as last event, got {other:?}"),
    }
}

fn assert_scan_has_progress(events: &[ScanProgressEvent]) {
    let found = events.iter().any(|e| {
        if let ScanProgressEvent::Progress {
            scanned_items,
            scanned_bytes,
            ..
        } = e
        {
            assert_u64_string_decimal(scanned_items);
            assert_u64_string_decimal(scanned_bytes);
            return true;
        }
        false
    });
    assert!(found, "expected at least one Progress event: {events:?}");
}

pub fn assert_scan_counters_decimal_strings(results: &ScanResultsDto) {
    assert!(!results.targets.is_empty(), "expected at least one target");
    for target in &results.targets {
        assert_u64_string_decimal(&target.total_bytes.0.to_string());
        assert_u64_string_decimal(&target.eligible_bytes.0.to_string());
    }
}

fn assert_u64_string_decimal(s: &str) {
    let parsed = s.parse::<u64>();
    assert!(parsed.is_ok(), "expected decimal u64 string, got {s}");
}

pub fn assert_clean_log_contains_intent_and_outcome(log_content: &str) {
    assert!(
        log_content.contains("\"record\":\"intent\""),
        "{log_content}"
    );
    assert!(
        log_content.contains("\"record\":\"outcome\""),
        "{log_content}"
    );
    assert!(
        log_content.contains("\"status\":\"deleted\""),
        "{log_content}"
    );
}

pub fn preview_rejects_missing_and_wrong_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    item_id: &str,
) {
    let none_err = commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![item_id.to_string()],
        None,
    )
    .unwrap_err();
    assert_invalid_input_err(none_err);
    let wrong_err = commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![item_id.to_string()],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("WRONG".to_string()),
        }),
    )
    .unwrap_err();
    assert_invalid_input_err(wrong_err);
}

pub fn preview_accepts_delete_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    item_id: &str,
) -> String {
    let resp = commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![item_id.to_string()],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!resp.preview_token.is_empty());
    resp.preview_token
}

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

pub fn assert_relaunch_cancel_preserved(ctx: &ElevationTestState, err: ContractError) {
    common_assert_invalid_or_input(err);
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

fn common_assert_invalid_or_input(err: ContractError) {
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}
