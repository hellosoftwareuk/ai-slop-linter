use std::{fs, path::Path, time::Duration};

use slop::{
    analyzer::analyze_file,
    discovery::{scan, ScanOptions},
    model::Language,
    scoring::build_report,
};

fn analyze_fixture(name: &str) -> slop::model::FileAnalysis {
    let root = Path::new("tests/fixtures");
    let path = root.join(name);
    let source = fs::read_to_string(&path).expect("fixture should be readable");
    analyze_file(&path, root, source.clone(), source.len() as u64)
        .expect("fixture should be analyzable")
}

fn analyze_inline(path: &str, source: &str) -> slop::model::FileAnalysis {
    analyze_file(
        Path::new(path),
        Path::new("."),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("inline source should be analyzable")
}

fn has_rule(analysis: &slop::model::FileAnalysis, rule: &str) -> bool {
    analysis.findings.iter().any(|finding| finding.rule == rule)
}

fn analyze_repository(sources: Vec<(String, String)>) -> slop::model::ScanReport {
    let analyses = sources
        .into_iter()
        .map(|(path, source)| {
            analyze_file(
                Path::new(&path),
                Path::new("."),
                source.clone(),
                source.len() as u64,
            )
            .expect("repository source should be analyzable")
        })
        .collect();
    build_report(Path::new("."), analyses, Duration::ZERO)
}

fn report_has_rule(report: &slop::model::ScanReport, rule: &str) -> bool {
    report.findings.iter().any(|finding| finding.rule == rule)
}

#[path = "analysis/basic.rs"]
mod basic;
#[path = "analysis/fixes.rs"]
mod fixes;
#[path = "analysis/flow.rs"]
mod flow;
#[path = "analysis/hcl.rs"]
mod hcl;
#[path = "analysis/repository.rs"]
mod repository;
#[path = "analysis/signals.rs"]
mod signals;
