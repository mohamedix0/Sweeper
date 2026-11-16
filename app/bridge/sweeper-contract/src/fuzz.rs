use crate::types::*;

/// Deterministic serde fuzz entrypoint for contract payload bytes.
/// Accepts arbitrary bytes and must never panic.
pub fn fuzz_contract_payload_bytes(data: &[u8]) {
    if let Ok(s) = std::str::from_utf8(data) {
        fuzz_from_str(s);
    }
    fuzz_from_slice(data);
}

fn fuzz_from_str(s: &str) {
    let _ = serde_json::from_str::<U64String>(s);
    let _ = serde_json::from_str::<TierDto>(s);
    let _ = serde_json::from_str::<RiskDto>(s);
    let _ = serde_json::from_str::<SkipReasonDto>(s);
    let _ = serde_json::from_str::<SkippedReasonDto>(s);
    let _ = serde_json::from_str::<ScanItemDto>(s);
    let _ = serde_json::from_str::<TargetScanResultDto>(s);
    let _ = serde_json::from_str::<ScanResultsDto>(s);
    let _ = serde_json::from_str::<StartScanResponse>(s);
    let _ = serde_json::from_str::<AnalyzerNodeDto>(s);
    let _ = serde_json::from_str::<AnalyzerProgressEvent>(s);
    let _ = serde_json::from_str::<StartAnalyzerResponse>(s);
    let _ = serde_json::from_str::<ValidateCustomPathResponse>(s);
    let _ = serde_json::from_str::<PreviewConfirmationsDto>(s);
    let _ = serde_json::from_str::<PreviewTargetBreakdownDto>(s);
    let _ = serde_json::from_str::<PreviewCleanResponse>(s);
    let _ = serde_json::from_str::<ExecuteCleanResponse>(s);
    let _ = serde_json::from_str::<ExplainItemResponse>(s);
    let _ = serde_json::from_str::<SettingsDto>(s);
    let _ = serde_json::from_str::<LogMetadataDto>(s);
    let _ = serde_json::from_str::<LogReadResponse>(s);
    let _ = serde_json::from_str::<ScanProgressEvent>(s);
    let _ = serde_json::from_str::<CleanProgressEvent>(s);
    let _ = serde_json::from_str::<ContractError>(s);
    let _ = serde_json::from_str::<ElevationStatusDto>(s);
    let _ = serde_json::from_str::<RelaunchResultDto>(s);
    let _ = serde_json::from_str::<Vec<String>>(s);
    let _ = serde_json::from_str::<Vec<TierDto>>(s);
}

fn fuzz_from_slice(data: &[u8]) {
    let _ = serde_json::from_slice::<U64String>(data);
    let _ = serde_json::from_slice::<ScanResultsDto>(data);
    let _ = serde_json::from_slice::<AnalyzerNodeDto>(data);
    let _ = serde_json::from_slice::<StartAnalyzerResponse>(data);
    let _ = serde_json::from_slice::<PreviewCleanResponse>(data);
    let _ = serde_json::from_slice::<ExecuteCleanResponse>(data);
    let _ = serde_json::from_slice::<CleanProgressEvent>(data);
    let _ = serde_json::from_slice::<ContractError>(data);
}
