use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Diagnostic, Finding, ScanReport};

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisBaseline {
    finding_identities: HashSet<String>,
    diagnostic_counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticDelta {
    pub kind: &'static str,
    pub path: String,
    pub count: usize,
    pub message: String,
    pub remediation_prompt: String,
}

#[derive(Debug)]
pub struct AnalysisDelta {
    pub findings: Vec<Finding>,
    pub diagnostics: Vec<DiagnosticDelta>,
}

impl AnalysisBaseline {
    pub fn capture(report: &ScanReport) -> Self {
        Self {
            finding_identities: report.findings.iter().map(finding_identity).collect(),
            diagnostic_counts: report
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic_key(&diagnostic.path, diagnostic.kind),
                        diagnostic.count,
                    )
                })
                .collect(),
        }
    }

    pub fn compare(&self, report: &ScanReport) -> AnalysisDelta {
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.points > 0.0 && self.is_new_finding(finding))
            .cloned()
            .collect();
        let diagnostics = report
            .diagnostics
            .iter()
            .filter_map(|diagnostic| self.diagnostic_delta(diagnostic))
            .collect();
        AnalysisDelta {
            findings,
            diagnostics,
        }
    }

    pub fn is_new_finding(&self, finding: &Finding) -> bool {
        !self.finding_identities.contains(&finding_identity(finding))
    }

    pub fn diagnostic_increase(&self, path: &str, kind: &str, current: usize) -> usize {
        let previous = self
            .diagnostic_counts
            .get(&diagnostic_key(path, kind))
            .copied()
            .unwrap_or_default();
        current.saturating_sub(previous)
    }

    fn diagnostic_delta(&self, diagnostic: &Diagnostic) -> Option<DiagnosticDelta> {
        let increase =
            self.diagnostic_increase(&diagnostic.path, diagnostic.kind, diagnostic.count);
        (increase > 0).then(|| DiagnosticDelta {
            kind: diagnostic.kind,
            path: diagnostic.path.clone(),
            count: increase,
            message: format!(
                "{increase} new syntax error{} reduce analysis coverage",
                if increase == 1 { "" } else { "s" }
            ),
            remediation_prompt: diagnostic.remediation_prompt.clone(),
        })
    }
}

fn finding_identity(finding: &Finding) -> String {
    format!("{}\0{}\0{}", finding.path, finding.rule, finding.message)
}

fn diagnostic_key(path: &str, kind: &str) -> String {
    format!("{path}\0{kind}")
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use crate::{model::Category, scoring::build_report};

    use super::*;

    #[test]
    fn existing_finding_identity_ignores_line_and_evidence_changes() {
        let first = finding(10, "61 lines");
        let improved = finding(14, "60 lines");
        assert_eq!(finding_identity(&first), finding_identity(&improved));
    }

    #[test]
    fn comparison_returns_only_new_findings_and_diagnostic_increases() {
        let mut before = empty_report();
        before.findings.push(finding(10, "61 lines"));
        before.diagnostics.push(Diagnostic {
            kind: "parse-error",
            path: "src/old.ts".to_owned(),
            count: 1,
            message: "one error".to_owned(),
            remediation_prompt: "repair it".to_owned(),
        });
        let baseline = AnalysisBaseline::capture(&before);

        let mut after = empty_report();
        after.findings.push(finding(20, "70 lines"));
        after.findings.push(Finding::new(
            "prefer-const",
            Category::Readability,
            1.0,
            ("src/new.ts".to_owned(), 1),
            ("binding can use const", "one binding"),
        ));
        after.diagnostics.push(Diagnostic {
            kind: "parse-error",
            path: "src/old.ts".to_owned(),
            count: 2,
            message: "two errors".to_owned(),
            remediation_prompt: "repair them".to_owned(),
        });

        let delta = baseline.compare(&after);
        assert_eq!(delta.findings.len(), 1);
        assert_eq!(delta.findings[0].rule, "prefer-const");
        assert_eq!(delta.diagnostics.len(), 1);
        assert_eq!(delta.diagnostics[0].count, 1);
    }

    fn finding(line: usize, evidence: &str) -> Finding {
        Finding::new(
            "long-function",
            Category::Size,
            4.0,
            ("src/app.ts".to_owned(), line),
            ("`loadApp` is too long", evidence),
        )
    }

    fn empty_report() -> ScanReport {
        build_report(Path::new("."), Vec::new(), Duration::ZERO)
    }
}
