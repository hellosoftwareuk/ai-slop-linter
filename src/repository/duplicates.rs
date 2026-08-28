use std::collections::{HashMap, HashSet};

use crate::model::{Category, CloneCandidate, FileAnalysis, Finding, Language};

type CloneKey = (Language, u64, usize);

pub(super) fn evaluate(files: &[FileAnalysis]) -> Vec<Finding> {
    let mut groups: HashMap<CloneKey, Vec<(&FileAnalysis, &CloneCandidate)>> = HashMap::new();
    for file in files
        .iter()
        .filter(|file| !is_test_path(&file.display_path))
    {
        for candidate in &file.clone_candidates {
            groups
                .entry((file.language, candidate.fingerprint, candidate.tokens))
                .or_default()
                .push((file, candidate));
        }
    }

    groups
        .into_values()
        .filter(|group| distinct_file_count(group) >= 2)
        .map(clone_finding)
        .collect()
}

fn distinct_file_count(group: &[(&FileAnalysis, &CloneCandidate)]) -> usize {
    group
        .iter()
        .map(|(file, _)| file.display_path.as_str())
        .collect::<HashSet<_>>()
        .len()
}

fn clone_finding(mut group: Vec<(&FileAnalysis, &CloneCandidate)>) -> Finding {
    group.sort_unstable_by(|left, right| {
        left.0
            .display_path
            .cmp(&right.0.display_path)
            .then_with(|| left.1.line.cmp(&right.1.line))
    });
    let (first_file, first) = group[0];
    let locations = group
        .iter()
        .skip(1)
        .take(4)
        .map(|(file, candidate)| format!("{}:{}", file.display_path, candidate.line))
        .collect::<Vec<_>>()
        .join(", ");
    Finding::new(
        "structural-clone",
        Category::Abstraction,
        (5.0 + (group.len() - 2) as f64 + first.tokens as f64 / 100.0).min(12.0),
        (first_file.display_path.clone(), first.line),
        (
            "Large code regions repeat the same structure with renamed values",
            format!(
                "{} equivalent regions across {} files; {} normalized tokens over lines {}-{}; other locations: {locations}",
                group.len(),
                distinct_file_count(&group),
                first.tokens,
                first.line,
                first.end_line,
            ),
        ),
    )
}

fn is_test_path(path: &str) -> bool {
    let normalized = path.to_ascii_lowercase();
    const TEST_DIRECTORIES: &[&str] = &["test", "tests", "fixtures"];
    const TEST_SUFFIXES: &[&str] = &[".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx", "_test.rs"];
    let test_directory = normalized
        .split('/')
        .any(|part| TEST_DIRECTORIES.contains(&part));
    test_directory
        || TEST_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_are_excluded_from_clone_groups() {
        assert!(is_test_path("tests/integration.rs"));
        assert!(is_test_path("src/orders/service.spec.ts"));
        assert!(is_test_path("src/parser_test.rs"));
        assert!(!is_test_path("src/orders/service.ts"));
    }
}
