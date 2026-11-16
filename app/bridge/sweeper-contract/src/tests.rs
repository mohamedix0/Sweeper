use crate::bindings;
use crate::types::*;
use std::fs;

#[test]
fn u64_string_serializes_as_decimal_string() {
    let v = U64String(9_007_199_254_740_993);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "\"9007199254740993\"");
    let de: U64String = serde_json::from_str(&json).unwrap();
    assert_eq!(de, v);
}

fn sample_scan_item() -> ScanItemDto {
    ScanItemDto {
        id: "t::1".to_string(),
        target_id: "user-temp".to_string(),
        path: r"C:\Users\X\AppData\Local\Temp\a.tmp".to_string(),
        size: U64String(123),
        mtime_unix_secs: U64String(456),
        age_eligible: true,
    }
}

#[test]
fn scan_item_serializes_u64_fields_as_strings() {
    let v = serde_json::to_value(sample_scan_item()).unwrap();
    assert_eq!(v["size"], serde_json::json!("123"));
    assert_eq!(v["mtimeUnixSecs"], serde_json::json!("456"));
    assert_eq!(v["ageEligible"], serde_json::json!(true));
}

#[test]
fn scan_item_roundtrips() {
    let item = sample_scan_item();
    let v = serde_json::to_value(&item).unwrap();
    let back: ScanItemDto = serde_json::from_value(v).unwrap();
    assert_eq!(back, item);
}

#[test]
fn clean_progress_event_serializes_u64_fields_as_strings() {
    let event = CleanProgressEvent {
        preview_token: "pt".to_string(),
        target_id: "t1".to_string(),
        completed_items: U64String(1),
        total_items: U64String(2),
        bytes_freed_so_far: U64String(3),
        status: "running".to_string(),
    };
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(v["completedItems"], serde_json::json!("1"));
    assert_eq!(v["totalItems"], serde_json::json!("2"));
    assert_eq!(v["bytesFreedSoFar"], serde_json::json!("3"));
}

fn sample_clean_response() -> ExecuteCleanResponse {
    ExecuteCleanResponse {
        bytes_freed: U64String(10),
        deleted_count: U64String(2),
        skipped_count: U64String(1),
        skipped_reasons: vec![SkippedReasonDto {
            reason: "Skipped: locked".to_string(),
            count: U64String(1),
        }],
        aborted: false,
        abort_reason: None,
        log_id: Some("log-1".to_string()),
    }
}

#[test]
fn execute_clean_response_serializes_counts_as_strings() {
    let vr = serde_json::to_value(sample_clean_response()).unwrap();
    assert_eq!(vr["bytesFreed"], serde_json::json!("10"));
    assert_eq!(vr["deletedCount"], serde_json::json!("2"));
    assert_eq!(vr["skippedCount"], serde_json::json!("1"));
}

#[test]
fn execute_clean_response_serializes_skip_reason_counts() {
    let vr = serde_json::to_value(sample_clean_response()).unwrap();
    assert_eq!(vr["skippedReasons"][0]["count"], serde_json::json!("1"));
    assert_eq!(vr["aborted"], serde_json::json!(false));
    assert_eq!(vr["logId"], serde_json::json!("log-1"));
}

fn sample_analyzer_progress_event() -> AnalyzerProgressEvent {
    AnalyzerProgressEvent {
        session_id: "a1".to_string(),
        scanned_dirs: U64String(2),
        scanned_files: U64String(3),
        scanned_bytes: U64String(4),
        current_path: Some(r"C:\Temp".to_string()),
    }
}

#[test]
fn analyzer_progress_event_serializes_u64_fields_as_strings() {
    let ev = serde_json::to_value(sample_analyzer_progress_event()).unwrap();
    assert_eq!(ev["sessionId"], serde_json::json!("a1"));
    assert_eq!(ev["scannedDirs"], serde_json::json!("2"));
    assert_eq!(ev["scannedFiles"], serde_json::json!("3"));
}

#[test]
fn analyzer_progress_event_serializes_bytes_and_path() {
    let ev = serde_json::to_value(sample_analyzer_progress_event()).unwrap();
    assert_eq!(ev["scannedBytes"], serde_json::json!("4"));
    assert_eq!(ev["currentPath"], serde_json::json!(r"C:\Temp"));
}

fn sample_analyzer_response() -> StartAnalyzerResponse {
    let node = AnalyzerNodeDto {
        id: "n1".to_string(),
        name: "Temp".to_string(),
        path: r"C:\Temp".to_string(),
        is_dir: true,
        size_bytes: U64String(10),
        item_count: U64String(2),
        risk: RiskDto::Safe,
        description: "desc".to_string(),
        reason: None,
        tier: Some(TierDto::Tier1),
        target_id: Some("user-temp".to_string()),
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: true,
        children: vec![],
    };
    StartAnalyzerResponse {
        session_id: "a1".to_string(),
        root_path: r"C:\Temp".to_string(),
        total_bytes: U64String(10),
        total_items: U64String(2),
        scanned_dirs: U64String(1),
        scanned_files: U64String(1),
        cancelled: false,
        warnings: vec![],
        root: node,
    }
}

#[test]
fn analyzer_response_serializes_totals_as_strings() {
    let v = serde_json::to_value(sample_analyzer_response()).unwrap();
    assert_eq!(v["totalBytes"], serde_json::json!("10"));
    assert_eq!(v["totalItems"], serde_json::json!("2"));
    assert_eq!(v["scannedDirs"], serde_json::json!("1"));
}

#[test]
fn analyzer_response_serializes_root_and_roundtrips() {
    let resp = sample_analyzer_response();
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["scannedFiles"], serde_json::json!("1"));
    assert_eq!(v["root"]["sizeBytes"], serde_json::json!("10"));
    assert_eq!(v["root"]["itemCount"], serde_json::json!("2"));
    let _roundtrip: StartAnalyzerResponse = serde_json::from_value(v).unwrap();
}

fn sample_scan_progress() -> ScanProgressEvent {
    ScanProgressEvent::Progress {
        session_id: "s1".to_string(),
        completed_targets: 1,
        total_targets: 3,
        current_target_id: Some("user-temp".to_string()),
        current_target_name: Some("User Temp".to_string()),
        scanned_items: "42".to_string(),
        scanned_bytes: "4096".to_string(),
    }
}

#[test]
fn scan_progress_event_serializes_core_fields() {
    let v = serde_json::to_value(sample_scan_progress()).unwrap();
    assert_eq!(v["kind"], serde_json::json!("progress"));
    assert_eq!(v["session_id"], serde_json::json!("s1"));
    assert_eq!(v["completed_targets"], serde_json::json!(1));
}

#[test]
fn scan_progress_event_serializes_counts_and_roundtrips() {
    let ev = sample_scan_progress();
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["total_targets"], serde_json::json!(3));
    assert_eq!(v["scanned_items"], serde_json::json!("42"));
    assert_eq!(v["scanned_bytes"], serde_json::json!("4096"));
    let back: ScanProgressEvent = serde_json::from_value(v).unwrap();
    assert_eq!(back, ev);
}

#[test]
fn scan_progress_cancelled_event_roundtrips() {
    let cancelled = ScanProgressEvent::Cancelled {
        session_id: "s1".to_string(),
    };
    let vc = serde_json::to_value(&cancelled).unwrap();
    assert_eq!(vc["kind"], serde_json::json!("cancelled"));
    assert_eq!(vc["session_id"], serde_json::json!("s1"));
    let _: ScanProgressEvent = serde_json::from_value(vc).unwrap();
}

#[test]
fn fuzz_entrypoint_never_panics_on_smoke_inputs() {
    for sample in [
        &b""[..],
        b"null",
        b"\"123\"",
        b"{\"bad\":true}",
        b"\xff\xfe\x00\x00",
    ] {
        crate::fuzz_contract_payload_bytes(sample);
    }
}

#[test]
fn export_ts_bindings_matches_committed_file() {
    let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("frontend")
        .join("src")
        .join("contract")
        .join("bindings.ts");
    let tmp = tempfile::NamedTempFile::new().unwrap();
    bindings::export_ts_bindings_to(tmp.path()).expect("export bindings");
    let generated = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        !generated.contains('\u{feff}'),
        "generated bindings must not contain UTF-8 BOM"
    );
    let existing = fs::read_to_string(&committed)
        .unwrap_or_else(|_| panic!("missing committed bindings at {}", committed.display()));
    assert!(
        !existing.contains('\u{feff}'),
        "committed bindings must not contain UTF-8 BOM"
    );
    let existing_normalized = existing.replace("\r\n", "\n");
    assert_eq!(
        generated, existing_normalized,
        "TypeScript bindings drift detected; re-export frontend/src/contract/bindings.ts"
    );
}
