//! Human-readable terminal formatter for [`DoctorReport`].

use crate::model::DoctorReport;

/// Formats a [`DoctorReport`] for readable CLI output with actionable remediations.
#[must_use]
pub fn render_human_report(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str("BIFRÖST System Compatibility Report (`bifrost doctor`)\n");
    out.push_str("======================================================\n");
    out.push_str(&format!("Target Platform:    {}\n", report.target_platform));
    out.push_str(&format!(
        "Overall Readiness:  {} ({} passed, {} warnings, {} failed)\n\n",
        report.overall, report.pass_count, report.warn_count, report.fail_count
    ));

    out.push_str("Checks:\n");
    for check in &report.checks {
        out.push_str(&format!(
            "  [{:<4}] {:<36} {}\n",
            check.status, check.name, check.summary
        ));
        out.push_str(&format!("         {}\n", check.detail));
    }

    let remediations: Vec<(&str, &str)> = report
        .checks
        .iter()
        .filter_map(|c| c.remediation.as_deref().map(|r| (c.name.as_str(), r)))
        .collect();

    if !remediations.is_empty() {
        out.push_str("\nRecommended Actions:\n");
        for (idx, (name, action)) in remediations.iter().enumerate() {
            out.push_str(&format!("  {}. [{}] {}\n", idx + 1, name, action));
        }
    }

    out.trim_end().to_string()
}
