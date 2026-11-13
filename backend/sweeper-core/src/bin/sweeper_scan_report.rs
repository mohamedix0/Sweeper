//! Dev-only read-only scan reporter (Gate E helper).
//! Performs a read-only Scan of enabled Tier 1 & Tier 2 targets and prints a structured report.

use sweeper_core::scanner;

fn main() {
    match scanner::scan_tier1_tier2(24) {
        Ok(report) => {
            print!("{}", scanner::format_report(&report));
            let violations = scanner::gate_e_violations(&report);
            if !violations.is_empty() {
                eprintln!("GATE E VIOLATIONS DETECTED:");
                for v in violations {
                    eprintln!("- {v}");
                }
                std::process::exit(2);
            }
        }
        Err(e) => {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
    }
}
