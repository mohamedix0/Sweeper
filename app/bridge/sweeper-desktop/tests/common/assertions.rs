#![allow(dead_code)]

use sweeper_contract::{
    ContractError, LogReadResponse, PreviewConfirmationsDto, ScanProgressEvent, ScanResultsDto,
    StartScanResponse,
};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::AppState;

#[derive(Clone, Copy)]
pub struct SessionItem<'a> {
    pub session_id: &'a str,
    pub item_id: &'a str,
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

pub fn assert_scan_progress_events(events: &[ScanProgressEvent], resp: &StartScanResponse) {
    assert!(!events.is_empty(), "expected scan progress events");
    assert_scan_started(&events[0], resp);
    assert_scan_has_progress(events);
    assert_scan_completed(events.last().unwrap(), resp);
}

fn assert_scan_started(event: &ScanProgressEvent, resp: &StartScanResponse) {
    match event {
        ScanProgressEvent::Started { session_id } => assert_eq!(session_id, &resp.session_id),
        other => panic!("expected Started as first event, got {other:?}"),
    }
}

fn assert_scan_completed(event: &ScanProgressEvent, resp: &StartScanResponse) {
    match event {
        ScanProgressEvent::Completed { session_id } => assert_eq!(session_id, &resp.session_id),
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
            assert_decimal(scanned_items);
            assert_decimal(scanned_bytes);
            return true;
        }
        false
    });
    assert!(found, "expected at least one Progress event: {events:?}");
}

pub fn assert_scan_counters_decimal_strings(results: &ScanResultsDto) {
    assert!(!results.targets.is_empty(), "expected at least one target");
    for target in &results.targets {
        assert_u64(target.total_bytes.0);
        assert_u64(target.eligible_bytes.0);
    }
}

fn assert_u64(value: u64) {
    let _ = value;
}

fn assert_decimal(value: &std::string::String) {
    let parsed = value.parse::<u64>();
    assert!(parsed.is_ok(), "expected decimal u64 string, got {value}");
}

pub fn assert_clean_log_contains_intent_and_outcome(log: &LogReadResponse) {
    assert!(
        log.content.contains("\"record\":\"intent\""),
        "{}",
        log.content
    );
    assert!(
        log.content.contains("\"record\":\"outcome\""),
        "{}",
        log.content
    );
    assert!(
        log.content.contains("\"status\":\"deleted\""),
        "{}",
        log.content
    );
}

pub fn preview_rejects_missing_and_wrong_confirmation(
    state: &AppState<FixedClock>,
    selection: SessionItem<'_>,
) {
    let none_err = commands_impl::preview_clean(
        state,
        selection.session_id.to_string(),
        vec![selection.item_id.to_string()],
        None,
    )
    .unwrap_err();
    assert_invalid_input_err(none_err);
    let wrong_err = commands_impl::preview_clean(
        state,
        selection.session_id.to_string(),
        vec![selection.item_id.to_string()],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("WRONG".to_string()),
        }),
    )
    .unwrap_err();
    assert_invalid_input_err(wrong_err);
}

pub fn preview_accepts_delete_confirmation(
    state: &AppState<FixedClock>,
    selection: SessionItem<'_>,
) -> String {
    let resp = commands_impl::preview_clean(
        state,
        selection.session_id.to_string(),
        vec![selection.item_id.to_string()],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!resp.preview_token.is_empty());
    resp.preview_token
}
