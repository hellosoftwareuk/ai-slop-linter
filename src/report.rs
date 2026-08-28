use std::io::{self, Write};

use anyhow::Result;

use crate::model::{Category, Language, ScanReport, Severity};

pub fn print_json(report: &ScanReport) -> Result<()> {
    let stdout = io::stdout();
    serde_json::to_writer_pretty(stdout.lock(), report)?;
    println!();
    Ok(())
}

pub fn print_text(report: &ScanReport, top: usize) {
    let mut output = io::BufWriter::new(io::stdout().lock());
    print_summary(&mut output, report);
    print_diagnostics(&mut output, report);

    if report.findings.is_empty() {
        let _ = writeln!(output, "\nNo slop hotspots found.");
        return;
    }

    let _ = writeln!(output, "\nTop findings:");
    for finding in report.findings.iter().take(top) {
        let severity = match finding.severity {
            Severity::High => "high",
            Severity::Warning => "warn",
            Severity::Note => "note",
        };
        let _ = writeln!(
            output,
            "  {:>4}  {}:{}  {}",
            severity, finding.path, finding.line, finding.message
        );
        let _ = writeln!(
            output,
            "        {} [{}; +{:.1}]",
            finding.evidence, finding.rule, finding.points
        );
        let _ = writeln!(output, "        LLM prompt: {}", finding.remediation_prompt);
    }

    if report.findings.len() > top {
        let _ = writeln!(
            output,
            "\n{} more finding{} hidden; use --top {} or --format json.",
            report.findings.len() - top,
            plural(report.findings.len() - top),
            report.findings.len()
        );
    }
}

fn print_diagnostics(output: &mut impl Write, report: &ScanReport) {
    if report.diagnostics.is_empty() {
        return;
    }
    let _ = writeln!(output, "\nParser diagnostics:");
    for diagnostic in &report.diagnostics {
        let _ = writeln!(
            output,
            "  {}  {}  {}",
            diagnostic.kind, diagnostic.path, diagnostic.message
        );
        let _ = writeln!(
            output,
            "        LLM prompt: {}",
            diagnostic.remediation_prompt
        );
    }
}

fn print_summary(output: &mut impl Write, report: &ScanReport) {
    let _ = writeln!(
        output,
        "Slop score: {}/100 ({})",
        report.score, report.rating
    );
    let _ = writeln!(
        output,
        "Scanned {} source file{} ({}) with {} non-empty lines in {} ms",
        format_count(report.files),
        plural(report.files),
        language_summary(report),
        format_count(report.lines),
        report.elapsed_ms
    );
    let _ = writeln!(
        output,
        "Debt density: {:.1} points/KLOC  |  {} finding{}  |  {} parse error{}",
        report.points_per_kloc,
        report.findings.len(),
        plural(report.findings.len()),
        report.parse_errors,
        plural(report.parse_errors),
    );
    let macro_trees = report.metrics.macro_invocations + report.metrics.macro_definitions;
    let _ = writeln!(
        output,
        "Repository graph: {} internal edges across {} directories  |  {} unresolved relative dependencies",
        format_count(report.repository_metrics.internal_dependencies),
        format_count(report.repository_metrics.directories),
        report.repository_metrics.unresolved_relative_dependencies,
    );
    if macro_trees > 0 {
        let _ = writeln!(
            output,
            "Rust macro syntax: {}/{} token trees analyzed  |  {} use a non-Rust DSL",
            report.metrics.macro_inputs_analyzed,
            macro_trees,
            report.metrics.macro_inputs_unresolved,
        );
    }

    if !report.category_scores.is_empty() {
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "Complexity {:>3}  Size {:>3}  Readability {:>3}  Abstraction {:>3}  Type safety {:>3}",
            category_score(report, Category::Complexity),
            category_score(report, Category::Size),
            category_score(report, Category::Readability),
            category_score(report, Category::Abstraction),
            category_score(report, Category::TypeSafety),
        );
        let _ = writeln!(
            output,
            "Architecture {:>3}  Structure {:>3}",
            category_score(report, Category::Architecture),
            category_score(report, Category::Structure),
        );
    }
}

fn language_summary(report: &ScanReport) -> String {
    let typescript = report
        .languages
        .get(&Language::TypeScript)
        .copied()
        .unwrap_or_default();
    let rust = report
        .languages
        .get(&Language::Rust)
        .copied()
        .unwrap_or_default();
    let terraform = report
        .languages
        .get(&Language::Terraform)
        .copied()
        .unwrap_or_default();
    let terragrunt = report
        .languages
        .get(&Language::Terragrunt)
        .copied()
        .unwrap_or_default();
    format!(
        "{} TypeScript, {} Rust, {} Terraform, {} Terragrunt",
        format_count(typescript),
        format_count(rust),
        format_count(terraform),
        format_count(terragrunt),
    )
}

fn category_score(report: &ScanReport, category: Category) -> u8 {
    report
        .category_scores
        .get(&category)
        .copied()
        .unwrap_or_default()
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(byte as char);
    }
    result
}
