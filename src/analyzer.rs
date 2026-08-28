mod behavior_rules;
mod clone_detection;
mod core;
mod rules;
mod rust;
mod rust_signals;
mod rust_syntax;
mod typescript;
mod typescript_signals;

use std::path::Path;

use anyhow::Result;

use crate::model::{FileAnalysis, Language};

pub fn analyze_file(path: &Path, root: &Path, source: String, bytes: u64) -> Result<FileAnalysis> {
    let language = Language::from_path(path)
        .ok_or_else(|| anyhow::anyhow!("unsupported source path '{}'", path.display()))?;
    let display_path = relative_path(path, root);
    let lines = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    let parsed = match language {
        Language::TypeScript => typescript::collect(path, &source)?,
        Language::Rust => rust::collect(&source),
    };
    let (metrics, mut findings) = rules::evaluate(&display_path, lines, &parsed.facts);
    findings.sort_unstable_by(|left, right| {
        right
            .points
            .total_cmp(&left.points)
            .then_with(|| left.line.cmp(&right.line))
    });

    Ok(FileAnalysis {
        path: path.to_path_buf(),
        display_path,
        language,
        bytes,
        lines,
        parse_errors: parsed.parse_errors,
        metrics,
        findings,
        dependencies: parsed.facts.dependencies,
        clone_candidates: parsed.facts.clone_candidates,
        top_level_statements: parsed.facts.top_level_statements,
    })
}

fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
