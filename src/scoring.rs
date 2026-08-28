use std::{collections::BTreeMap, path::Path, time::Duration};

use crate::model::{AstMetrics, Category, Diagnostic, FileAnalysis, FileSummary, ScanReport};

// A score of 63 corresponds to 75 debt points/KLOC. This keeps ordinary
// repositories spread across the scale while still making dense hotspots loud.
const DENSITY_SCALE: f64 = 75.0;

pub fn build_report(root: &Path, analyses: Vec<FileAnalysis>, elapsed: Duration) -> ScanReport {
    let files = analyses.len();
    let lines: usize = analyses.iter().map(|file| file.lines).sum();
    let bytes: u64 = analyses.iter().map(|file| file.bytes).sum();
    let parse_errors: usize = analyses.iter().map(|file| file.parse_errors).sum();
    let diagnostics = parser_diagnostics(&analyses);
    let mut metrics = AstMetrics::default();
    let mut findings = Vec::new();
    let mut hotspots = Vec::new();
    let mut category_points = BTreeMap::new();
    let mut languages = BTreeMap::new();
    let repository_analysis = crate::repository::analyze(root, &analyses);

    for analysis in analyses {
        *languages.entry(analysis.language).or_insert(0) += 1;
        metrics.add_assign(&analysis.metrics);
        let file_points: f64 = analysis.findings.iter().map(|finding| finding.points).sum();
        if file_points > 0.0 {
            let density = file_points * 1_000.0 / analysis.lines.max(100) as f64;
            hotspots.push(FileSummary {
                path: analysis.display_path,
                score: score_from_density(density),
                lines: analysis.lines,
                points: round_one(file_points),
            });
        }
        for finding in analysis.findings {
            *category_points.entry(finding.category).or_insert(0.0) += finding.points;
            findings.push(finding);
        }
    }

    for finding in repository_analysis.findings {
        *category_points.entry(finding.category).or_insert(0.0) += finding.points;
        findings.push(finding);
    }

    sort_findings(&mut findings);
    sort_hotspots(&mut hotspots);

    let debt_points: f64 = findings.iter().map(|finding| finding.points).sum();
    let effective_lines = lines.max(500) as f64;
    let points_per_kloc = debt_points * 1_000.0 / effective_lines;
    let score = score_from_density(points_per_kloc);
    let category_scores = category_scores(&category_points, effective_lines);

    ScanReport {
        root: root.to_string_lossy().replace('\\', "/"),
        score,
        rating: rating(score),
        debt_points: round_one(debt_points),
        points_per_kloc: round_one(points_per_kloc),
        elapsed_ms: elapsed.as_millis(),
        files,
        languages,
        lines,
        bytes,
        parse_errors,
        diagnostics,
        metrics,
        repository_metrics: repository_analysis.metrics,
        category_scores,
        hotspots,
        findings,
    }
}

fn parser_diagnostics(analyses: &[FileAnalysis]) -> Vec<Diagnostic> {
    analyses
        .iter()
        .filter(|analysis| analysis.parse_errors > 0)
        .map(|analysis| Diagnostic {
            kind: "parse-error",
            path: analysis.display_path.clone(),
            count: analysis.parse_errors,
            message: format!(
                "{} syntax error(s) reduce analysis coverage",
                analysis.parse_errors
            ),
            remediation_prompt: crate::remediation::parser_prompt(
                &analysis.display_path,
                analysis.language,
                analysis.parse_errors,
            ),
        })
        .collect()
}

fn sort_findings(findings: &mut [crate::model::Finding]) {
    findings.sort_unstable_by(|left, right| {
        right
            .points
            .total_cmp(&left.points)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line.cmp(&right.line))
    });
}

fn sort_hotspots(hotspots: &mut [FileSummary]) {
    hotspots.sort_unstable_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.points.total_cmp(&left.points))
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn category_scores(
    category_points: &BTreeMap<Category, f64>,
    effective_lines: f64,
) -> BTreeMap<Category, u8> {
    Category::ALL
        .into_iter()
        .map(|category| {
            let points = category_points.get(&category).copied().unwrap_or_default();
            let density = points * 1_000.0 / effective_lines;
            (category, score_from_density(density))
        })
        .collect()
}

fn score_from_density(points_per_kloc: f64) -> u8 {
    let score = 100.0 * (1.0 - (-points_per_kloc / DENSITY_SCALE).exp());
    score.round().clamp(0.0, 100.0) as u8
}

fn rating(score: u8) -> &'static str {
    match score {
        0..=14 => "low",
        15..=34 => "moderate",
        35..=59 => "high",
        _ => "severe",
    }
}

fn round_one(value: f64) -> f64 {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_is_bounded_and_monotonic() {
        assert_eq!(score_from_density(0.0), 0);
        assert!(score_from_density(10.0) < score_from_density(20.0));
        assert!(score_from_density(1_000.0) <= 100);
    }
}
