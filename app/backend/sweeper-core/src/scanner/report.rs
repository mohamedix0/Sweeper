use super::*;

pub fn format_report(report: &ScanReport) -> String {
    let mut out = String::new();
    push_report_header(&mut out, report);
    for t in &report.targets {
        push_target_report(&mut out, t);
    }
    out
}

fn push_report_header(out: &mut String, report: &ScanReport) {
    out.push_str(&format!(
        "Sweeper read-only scan report (min_age_hours={})\n",
        report.min_age_hours
    ));
    out.push_str(&format!(
        "started_unix_secs={}\n\n",
        report.started_unix_secs
    ));
}

fn push_target_report(out: &mut String, t: &TargetScanResult) {
    push_target_summary(out, t);
    push_target_resolved_paths(out, t);
    push_target_totals(out, t);
    push_target_skip_details(out, t);
    out.push('\n');
}

fn push_target_summary(out: &mut String, t: &TargetScanResult) {
    out.push_str(&format!(
        "target_id={} tier={:?} risk={:?}\n",
        t.target_id, t.tier, t.risk
    ));
    out.push_str(&format!("  name={}\n", t.target_name));
    out.push_str(&format!("  scanned={}\n", t.scanned));
    if let Some(r) = &t.status_reason {
        out.push_str(&format!("  status={r}\n", r = r));
    }
}

fn push_target_resolved_paths(out: &mut String, t: &TargetScanResult) {
    if t.resolved_paths.is_empty() {
        out.push_str("  resolved_paths=[]\n");
        return;
    }
    out.push_str("  resolved_paths:\n");
    for p in &t.resolved_paths {
        out.push_str(&format!("    - {p}\n"));
    }
}

fn push_target_totals(out: &mut String, t: &TargetScanResult) {
    out.push_str(&format!(
        "  total_bytes={} eligible_bytes={} file_count={} eligible_count={} skipped_count={}\n",
        t.total_bytes, t.eligible_bytes, t.file_count, t.eligible_count, t.skipped_count
    ));
}

fn push_target_skip_details(out: &mut String, t: &TargetScanResult) {
    push_named_list(
        out,
        "  skipped_reasons",
        t.skipped_reasons.iter(),
        |(k, v)| format!("    - {k}: {v}\n"),
    );
    push_named_list(
        out,
        "  skipped_diagnostics",
        t.skipped_diagnostics.iter(),
        |d| format!("    - {d}\n"),
    );
}

fn push_named_list<T, I, F>(out: &mut String, label: &str, items: I, render: F)
where
    I: Iterator<Item = T>,
    F: Fn(T) -> String,
{
    let mut iter = items.peekable();
    if iter.peek().is_none() {
        return;
    }
    out.push_str(label);
    out.push_str(":\n");
    for item in iter {
        out.push_str(&render(item));
    }
}
