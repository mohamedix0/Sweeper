//! Unit tests for the remover orchestration and its extracted helpers.

use sweeper_winfs::ReparseTagClass;
use tempfile::tempdir;

use super::ancestors::{
    ANCESTOR_CHAIN_CACHE, ANCESTOR_CHAIN_CACHE_CAPACITY, ANCESTOR_CHAIN_RETAINED_COST,
    ANCESTOR_PREFIX_CHECKS,
};
use super::revalidation::path_has_link_point_ancestor;
use super::*;

#[test]
fn handle_final_path_divergence_from_confirmed_path_aborts() {
    // Residual D1 TOCTOU: the handle was opened while `sub` was a junction
    // to `keep`; the junction was swapped back to an ordinary directory
    // before revalidation. The ancestor re-check alone now sees no Link
    // Point, but the handle still targets the unselected `keep/victim.txt`.
    let target_root = r"C:\Temp\d1";
    let item = validated_item_for_tests(
        "d1-race",
        "test-target",
        target_root,
        r"C:\Temp\d1\sub\victim.txt",
        14,
        Some(0),
        ValidatedSource::CustomPath,
    );
    let inspection = sweeper_winfs::HandleInspection {
        final_path: r"C:\Temp\d1\keep\victim.txt".to_string(),
        identity: sweeper_winfs::FileIdentity {
            volume_serial: 1,
            file_index: 1,
        },
        attributes: 0x80, // FILE_ATTRIBUTE_NORMAL
        size: 14,
        reparse: ReparseTagClass::NotReparse,
        reparse_tag: 0,
        last_write_time: (0, 0),
    };
    let opts = CleanOptions {
        preview_estimated_bytes: 1024 * 1024,
        min_age: Duration::from_secs(0),
        now: SystemTime::now(),
        journal_base_dir: None,
    };

    let reason = revalidate_handle(&RevalidateRequest {
        item: &item,
        requested_path: item.path(),
        inspection: &inspection,
        opts: &opts,
        allow_delete_root: false,
        ignore_emptied_dir_age: false,
    })
    .expect_err("divergent handle final path must abort revalidation");
    assert!(
        reason.contains("escape"),
        "reason must classify as an abort: {reason}"
    );
}

#[test]
fn ancestor_chain_cache_precomputes_levels_and_stays_bounded() {
    let dir = CanonicalPath::parse_lexical(r"C:\Temp\a\b\c").unwrap();
    with_ancestor_chain(&dir, |chain| {
        let paths: Vec<&str> = chain.paths.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![r"C:\Temp", r"C:\Temp\a", r"C:\Temp\a\b", r"C:\Temp\a\b\c"]
        );
    });

    // The cache is a bounded optimization: it may not grow without limit.
    for i in 0..(ANCESTOR_CHAIN_CACHE_CAPACITY * 3) {
        let p = CanonicalPath::parse_lexical(&format!(r"C:\Temp\level{i}")).unwrap();
        with_ancestor_chain(&p, |_| ());
    }
    assert!(ancestor_chain_cache_len() <= ANCESTOR_CHAIN_CACHE_CAPACITY);
}

#[test]
fn ancestor_chain_cache_shares_siblings_and_revalidates_handles_after_tamper() {
    use sweeper_testkit::{create_junction, Sandbox};

    // Reset this thread's cache so the build count is unambiguous.
    ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
    ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));
    ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(0));

    let sb = Sandbox::new();
    let parent = sb.create_dir("target/level0").unwrap();
    let level = parent.join("level1");
    let level_canon = CanonicalPath::parse_lexical(&level.to_string_lossy()).unwrap();
    let mirror = sb.create_dir("mirror").unwrap();

    // Per-item checks exclude the final path, so all sibling files map to
    // the same parent directory level and must share a single cache entry.
    for i in 0..64 {
        let sibling =
            CanonicalPath::parse_lexical(&level.join(format!("sibling{i}.bin")).to_string_lossy())
                .unwrap();
        assert!(!path_has_link_point_ancestor(&sibling, &level_canon, true).unwrap());
    }
    assert_eq!(
        ancestor_chain_cache_len(),
        1,
        "siblings must share one entry"
    );
    assert_eq!(
        ancestor_chain_build_count(),
        1,
        "chain built once per level"
    );

    // Tamper after the cache is warm: the level becomes a Link Point. The
    // cached strings are unchanged, but each call opens and inspects the
    // ancestor handle again, so the Link Point is detected with no rebuild.
    create_junction(&level, &mirror).unwrap();
    assert!(path_has_link_point_ancestor(&level_canon, &level_canon, false).unwrap());
    assert_eq!(
        ancestor_chain_build_count(),
        1,
        "warm cache must be reused, not trusted as a verdict"
    );
}

#[test]
fn ancestor_chain_cache_bounds_complete_cost_of_deep_chains() {
    ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
    ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));

    let mut deep = String::from(r"C:\Temp");
    for i in 0..200 {
        deep.push_str(&format!(r"\level{i}"));
    }
    let deep_canon = CanonicalPath::parse_lexical(&deep).unwrap();
    assert!(
        ancestor_chain_cost(deep_canon.segment_count()) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET,
        "fixture must exceed the retained-cost budget"
    );

    for _ in 0..8 {
        with_ancestor_chain(&deep_canon, |chain| {
            assert_eq!(chain.paths.len(), deep_canon.segment_count());
        });
    }
    assert_eq!(
        ancestor_chain_cache_len(),
        0,
        "deep chains must not be retained"
    );
    assert_eq!(ancestor_chain_retained_cost(), 0);

    // Ordinary shallow directories remain cacheable within the budget.
    let shallow = CanonicalPath::parse_lexical(r"C:\Temp\a\b\c").unwrap();
    with_ancestor_chain(&shallow, |_| ());
    assert_eq!(ancestor_chain_cache_len(), 1);
    assert!(ancestor_chain_retained_cost() <= ANCESTOR_CHAIN_RETAINED_COST_BUDGET);
}

#[test]
fn deep_ancestor_chain_is_streamed_without_materialising_prefixes() {
    use sweeper_testkit::Sandbox;

    ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
    ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));
    ANCESTOR_PREFIX_CHECKS.with(|checks| checks.set(0));

    let sb = Sandbox::new();
    // 185 real levels exceed the cached cost budget (depth >= 181).
    let deep = sweeper_testkit::create_deep_tree(sb.root(), 185).unwrap();
    let deep_canon = CanonicalPath::from_existing_path(&deep.to_string_lossy())
        .unwrap()
        .into_canonical();
    assert!(ancestor_chain_cost(deep_canon.segment_count()) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET);

    assert!(!path_has_link_point_ancestor(&deep_canon, &deep_canon, false).unwrap());
    assert_eq!(
        ANCESTOR_PREFIX_CHECKS.with(|checks| checks.get()),
        deep_canon.segment_count(),
        "every prefix must be inspected one at a time"
    );
    assert_eq!(ancestor_chain_cache_len(), 0);
    assert_eq!(ancestor_chain_retained_cost(), 0);
}

#[test]
fn journal_failure_after_breaker_keeps_first_abort_reason() {
    let dir = tempdir().unwrap();
    let mut journal = JournalWriter::open_in(dir.path()).unwrap();
    let mut report = new_clean_report(2);
    report.aborted = true;
    report.abort_reason = Some("aborted: 120% circuit breaker exceeded".to_string());

    let report = stop_after_journal_failure(
        &mut journal,
        report,
        CleanError::Invariant("later skip write failed".to_string()),
        &[],
    );

    assert_eq!(
        report.abort_reason.as_deref(),
        Some("aborted: 120% circuit breaker exceeded"),
        "the first stop reason must be preserved"
    );
    assert!(
        report
            .journal_errors
            .iter()
            .any(|e| e.contains("clean stopped")),
        "the later journal failure must still be recorded: {:?}",
        report.journal_errors
    );
}

#[test]
fn breaker_journal_failure_preserves_breaker_reason_and_records_error() {
    // A breaker stop whose own Skipped-outcome write fails must keep the
    // breaker reason (apply_process_outcome never ran) and still record the
    // journal failure so a partial report reaches the caller.
    let dir = tempdir().unwrap();
    let mut journal = JournalWriter::open_in(dir.path()).unwrap();
    let report = new_clean_report(1);

    let report = stop_after_journal_failure(
        &mut journal,
        report,
        CleanError::BreakerJournalFailure {
            reason: "aborted: 120% circuit breaker exceeded".to_string(),
            detail: "disk full".to_string(),
        },
        &[],
    );

    assert!(report.aborted);
    assert_eq!(
        report.abort_reason.as_deref(),
        Some("aborted: 120% circuit breaker exceeded"),
        "the carried breaker reason must be kept as the abort reason"
    );
    assert!(
        report
            .journal_errors
            .iter()
            .any(|e| e.contains("clean stopped") && e.contains("disk full")),
        "the journal failure must still be recorded: {:?}",
        report.journal_errors
    );
}

#[test]
fn journal_failure_as_first_stop_sets_abort_reason() {
    let dir = tempdir().unwrap();
    let mut journal = JournalWriter::open_in(dir.path()).unwrap();
    let report = new_clean_report(1);

    let report = stop_after_journal_failure(
        &mut journal,
        report,
        CleanError::Invariant("first stop".to_string()),
        &[],
    );

    assert!(report.aborted);
    assert!(
        report
            .abort_reason
            .as_deref()
            .unwrap_or("")
            .contains("first stop"),
        "the journal failure must become the reason when it caused the stop"
    );
}

#[test]
fn contents_result_carries_deleted_any() {
    let mut state = ContentsState::new("C:\\A\\B\\C");
    assert!(!state.deleted().deleted_any);
    state.deleted_any = true;
    assert!(state.deleted().deleted_any);
    assert!(state.aborted("stop".into()).deleted_any);
    assert!(state.skipped("skip".into()).deleted_any);
}

#[test]
fn stop_outcomes_carry_completed_bytes() {
    let mut state = ContentsState::new("C:\\A\\B\\C");
    state.deleted_bytes = 9;
    let aborted = state.aborted("stop".to_string());
    assert!(matches!(aborted.outcome, ProcessOutcome::Aborted(_, 9)));
    let skipped = state.skipped("skip".to_string());
    assert!(matches!(skipped.outcome, ProcessOutcome::Skipped(_, 9)));
}

fn partial_item() -> ValidatedItem {
    validated_item_for_tests(
        "i",
        "t",
        "C:\\A\\B\\C",
        "C:\\A\\B\\C\\child",
        0,
        None,
        ValidatedSource::CustomPath,
    )
}

#[test]
fn fail_after_partial_reports_zero_byte_deletion_when_work_occurred() {
    // A zero-byte file is still a deletion; the decision must not depend
    // on a positive byte count.
    let item = partial_item();
    let mut state = ContentsState::new("C:\\A\\B\\C");
    state.deleted_any = true;
    let mut report = new_clean_report(1);
    let result: Result<(), CleanError> = fail_after_partial(
        &item,
        &state,
        &mut report,
        CleanError::Invariant("boom".into()),
    );
    assert!(result.is_err());
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].status, "deleted");
    assert_eq!(report.items[0].bytes_deleted, 0);
}

#[test]
fn fail_after_partial_skips_result_when_no_entry_was_deleted() {
    // A first-intent failure before any deletion must not fabricate a
    // Deleted result.
    let item = partial_item();
    let state = ContentsState::new("C:\\A\\B\\C");
    let mut report = new_clean_report(1);
    let result: Result<(), CleanError> = fail_after_partial(
        &item,
        &state,
        &mut report,
        CleanError::Invariant("boom".into()),
    );
    assert!(result.is_err());
    assert!(report.items.is_empty());
}
