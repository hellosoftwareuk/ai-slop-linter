mod git;

use std::{io, path::Path, time::Instant};

use anyhow::Result;
use serde::Serialize;

use crate::{
    delta::{AnalysisBaseline, DiagnosticDelta},
    discovery::{scan, ScanOptions},
    model::{Finding, ScanReport, Severity},
    scoring::build_report,
};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    WorkingTree,
    Staged,
}

#[derive(Debug, Serialize)]
pub struct DiffReport {
    pub root: String,
    pub comparison: DiffScope,
    pub base: String,
    pub base_commit: Option<String>,
    pub files_changed: usize,
    pub elapsed_ms: u128,
    pub new_debt_points: f64,
    pub findings: Vec<Finding>,
    pub diagnostics: Vec<DiagnosticDelta>,
    #[serde(skip)]
    annotation_prefix: String,
}

impl DiffReport {
    pub fn has_new_debt(&self) -> bool {
        !self.findings.is_empty() || !self.diagnostics.is_empty()
    }
}

pub fn analyze(path: &Path, base: Option<&str>, staged: bool) -> Result<DiffReport> {
    let started = Instant::now();
    let context = git::RepositoryContext::discover(path)?;
    let base_name = base.unwrap_or("HEAD").to_owned();
    let base_commit = git::resolve_commit(&context, &base_name, base.is_some())?;
    let options = scan_options();

    let baseline_report = match base_commit.as_deref() {
        Some(commit) => git::snapshot_report(&context, commit, &options)?,
        None => empty_report(&context.scan_path),
    };
    let current_report = if staged {
        let index_tree = git::index_tree(&context)?;
        git::snapshot_report(&context, &index_tree, &options)?
    } else {
        scan_report(&context.scan_path, &options)?
    };
    let delta = AnalysisBaseline::capture(&baseline_report).compare(&current_report);
    let new_debt_points = round_one(delta.findings.iter().map(|finding| finding.points).sum());
    let files_changed = git::changed_file_count(&context, base_commit.as_deref(), staged)?;

    Ok(DiffReport {
        root: display_path(&context.scan_path),
        comparison: if staged {
            DiffScope::Staged
        } else {
            DiffScope::WorkingTree
        },
        base: base_name,
        base_commit,
        files_changed,
        elapsed_ms: started.elapsed().as_millis(),
        new_debt_points,
        findings: delta.findings,
        diagnostics: delta.diagnostics,
        annotation_prefix: context.annotation_prefix(),
    })
}

pub fn print_json(report: &DiffReport) -> Result<()> {
    serde_json::to_writer_pretty(io::stdout().lock(), report)?;
    println!();
    Ok(())
}

pub fn print_text(report: &DiffReport, top: usize) {
    let scope = match report.comparison {
        DiffScope::WorkingTree => "working tree",
        DiffScope::Staged => "staged index",
    };
    println!(
        "Slop diff: {} changed file{} against {} ({scope}) in {} ms",
        report.files_changed,
        plural(report.files_changed),
        report.base,
        report.elapsed_ms
    );
    println!(
        "New debt: {:.1} points  |  {} finding{}  |  {} syntax error{}",
        report.new_debt_points,
        report.findings.len(),
        plural(report.findings.len()),
        diagnostic_count(report),
        plural(diagnostic_count(report))
    );
    if !report.has_new_debt() {
        println!("\nNo new slop introduced.");
        return;
    }

    for diagnostic in report.diagnostics.iter().take(top) {
        println!(
            "\n  error  {}  {}\n         LLM prompt: {}",
            diagnostic.path, diagnostic.message, diagnostic.remediation_prompt
        );
    }
    let remaining = top.saturating_sub(report.diagnostics.len());
    for finding in report.findings.iter().take(remaining) {
        println!(
            "\n  {:>5}  {}:{}  {}\n         {} [{}; +{:.1}]\n         LLM prompt: {}",
            severity_name(finding.severity),
            finding.path,
            finding.line,
            finding.message,
            finding.evidence,
            finding.rule,
            finding.points,
            finding.remediation_prompt
        );
    }
    let issue_count = report.findings.len() + report.diagnostics.len();
    if issue_count > top {
        println!(
            "\n{} more issue{} hidden; use --top {} or --format json.",
            issue_count - top,
            plural(issue_count - top),
            issue_count
        );
    }
}

pub fn print_github(report: &DiffReport) {
    for diagnostic in &report.diagnostics {
        println!(
            "::error file={},title={}::{}",
            github_property(&annotation_path(report, &diagnostic.path)),
            github_property("Slop syntax error"),
            github_message(&format!(
                "{}\n{}",
                diagnostic.message, diagnostic.remediation_prompt
            ))
        );
    }
    for finding in &report.findings {
        let level = if matches!(finding.severity, Severity::High) {
            "error"
        } else {
            "warning"
        };
        println!(
            "::{level} file={},line={},title={}::{}",
            github_property(&annotation_path(report, &finding.path)),
            finding.line,
            github_property(&format!("Slop [{}]", finding.rule)),
            github_message(&format!(
                "{} — {}\n{}",
                finding.message, finding.evidence, finding.remediation_prompt
            ))
        );
    }
    if report.has_new_debt() {
        println!(
            "Slop diff failed: {} new finding{} and {} new syntax error{}.",
            report.findings.len(),
            plural(report.findings.len()),
            diagnostic_count(report),
            plural(diagnostic_count(report))
        );
    } else {
        println!("Slop diff passed: no new maintainability debt.");
    }
}

fn scan_report(path: &Path, options: &ScanOptions) -> Result<ScanReport> {
    let started = Instant::now();
    let analyses = scan(path, options)?;
    Ok(build_report(path, analyses, started.elapsed()))
}

fn empty_report(path: &Path) -> ScanReport {
    build_report(path, Vec::new(), std::time::Duration::ZERO)
}

fn scan_options() -> ScanOptions {
    ScanOptions {
        include_declarations: false,
        respect_ignores: true,
        max_file_bytes: 2_000_000,
        threads: 0,
    }
}

fn diagnostic_count(report: &DiffReport) -> usize {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.count)
        .sum()
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Note => "note",
        Severity::Warning => "warn",
        Severity::High => "high",
    }
}

fn github_property(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

fn github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn annotation_path(report: &DiffReport, path: &str) -> String {
    if report.annotation_prefix.is_empty() {
        path.to_owned()
    } else {
        format!("{}/{path}", report.annotation_prefix)
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
}

fn round_one(value: f64) -> f64 {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_commands_escape_properties_and_messages() {
        assert_eq!(github_property("a:b,c%"), "a%3Ab%2Cc%25");
        assert_eq!(github_message("first\nsecond%"), "first%0Asecond%25");
    }
}
