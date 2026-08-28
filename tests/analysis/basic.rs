use super::*;

#[test]
fn clean_typescript_has_no_findings() {
    let analysis = analyze_fixture("clean.ts");
    assert_eq!(analysis.parse_errors, 0);
    assert!(analysis.findings.is_empty());
}

#[test]
fn tsx_is_parsed_without_special_configuration() {
    let analysis = analyze_fixture("component.tsx");
    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.functions, 1);
}

#[test]
fn clean_rust_uses_the_same_structural_analysis() {
    let analysis = analyze_fixture("clean.rs");

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.functions, 1);
    assert_eq!(analysis.metrics.structs, 1);
    assert_eq!(analysis.metrics.enums, 1);
    assert_eq!(analysis.metrics.traits, 1);
    assert!(analysis.findings.is_empty());
}

#[test]
fn sloppy_rust_exposes_shared_maintainability_signals() {
    let analysis = analyze_fixture("sloppy.rs");
    let rules = analysis
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();

    assert_eq!(analysis.parse_errors, 0);
    for expected in [
        "long-function",
        "complex-function",
        "deep-nesting",
        "parameter-bundle",
        "vague-names",
        "wrapper-cluster",
    ] {
        assert!(
            rules.contains(&expected),
            "missing rule {expected:?}: {rules:?}"
        );
    }
}

#[test]
fn folder_scan_discovers_both_languages() {
    let analyses = scan(
        Path::new("tests/fixtures"),
        &ScanOptions {
            include_declarations: false,
            respect_ignores: true,
            max_file_bytes: 2_000_000,
            threads: 1,
        },
    )
    .expect("fixtures should be scannable");

    let typescript = analyses
        .iter()
        .filter(|analysis| analysis.language == Language::TypeScript)
        .count();
    let rust = analyses
        .iter()
        .filter(|analysis| analysis.language == Language::Rust)
        .count();
    assert_eq!((typescript, rust), (6, 2));
}

#[test]
fn parent_scan_skips_fixture_directories() {
    let analyses = scan(
        Path::new("tests"),
        &ScanOptions {
            include_declarations: false,
            respect_ignores: true,
            max_file_bytes: 2_000_000,
            threads: 1,
        },
    )
    .expect("test sources should be scannable");

    assert!(analyses.iter().all(|analysis| !analysis
        .path
        .components()
        .any(|part| part.as_os_str() == "fixtures")));
}

#[test]
fn invalid_rust_is_reported_as_a_parse_error() {
    let analysis = analyze_file(
        Path::new("invalid.rs"),
        Path::new("."),
        "fn unfinished(".to_owned(),
        14,
    )
    .expect("parse failures should be represented in the analysis");

    assert!(analysis.parse_errors > 0);
    assert!(analysis.findings.is_empty());
}

#[test]
fn invalid_rust_still_produces_findings_from_valid_regions() {
    let source = r#"
fn tangled(data: bool, item: bool, value: bool, result: bool, temp: bool, tmp: bool) {
    if data {
        if item {
            if value {
                if result {
                    if temp {
                        if tmp {}
                    }
                }
            }
        }
    }
}

fn unfinished(
"#;
    let analysis = analyze_file(
        Path::new("partial.rs"),
        Path::new("."),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("invalid Rust should still yield a partial analysis");
    let rules = analysis
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();

    assert!(analysis.parse_errors > 0);
    assert!(rules.contains(&"complex-function"), "rules: {rules:?}");
    assert!(rules.contains(&"deep-nesting"), "rules: {rules:?}");
    assert!(rules.contains(&"parameter-bundle"), "rules: {rules:?}");
}

#[test]
fn rust_inside_macro_token_trees_is_analyzed() {
    let source = r#"
fn host() {
    quote! {
        fn generated(data: bool, item: bool, value: bool, result: bool, temp: bool, tmp: bool) {
            if data {
                if item {
                    if value {
                        if result {
                            if temp {
                                if tmp {}
                            }
                        }
                    }
                }
            }
        }
    };
}
"#;
    let analysis = analyze_file(
        Path::new("macro.rs"),
        Path::new("."),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("macro token trees should be analyzable");
    let rules = analysis
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.macro_invocations, 1);
    assert_eq!(analysis.metrics.macro_inputs_analyzed, 1);
    assert_eq!(analysis.metrics.macro_inputs_unresolved, 0);
    assert!(rules.contains(&"complex-function"), "rules: {rules:?}");
    assert!(rules.contains(&"deep-nesting"), "rules: {rules:?}");
    assert!(rules.contains(&"parameter-bundle"), "rules: {rules:?}");
}

#[test]
fn macro_rule_transcribers_are_analyzed_once_at_the_definition() {
    let source = r#"
macro_rules! generate {
    () => {
        fn generated(data: bool, item: bool, value: bool, result: bool, temp: bool, tmp: bool) {
            if data {
                if item {
                    if value {
                        if result {
                            if temp {
                                if tmp {}
                            }
                        }
                    }
                }
            }
        }
    };
}
"#;
    let analysis = analyze_file(
        Path::new("macro_rules.rs"),
        Path::new("."),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("macro transcribers should be analyzable");

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.macro_definitions, 1);
    assert_eq!(analysis.metrics.macro_inputs_analyzed, 1);
    assert!(analysis
        .findings
        .iter()
        .any(|finding| finding.rule == "complex-function"));
}

#[test]
fn non_rust_macro_dsls_are_reported_as_coverage_not_parse_errors() {
    let source = "fn query() { domain!(alpha => @ beta); }";
    let analysis = analyze_file(
        Path::new("dsl.rs"),
        Path::new("."),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("macro DSLs should not break the file scan");

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.macro_invocations, 1);
    assert_eq!(analysis.metrics.macro_inputs_analyzed, 0);
    assert_eq!(analysis.metrics.macro_inputs_unresolved, 1);
}

#[test]
fn sloppy_typescript_exposes_distinct_maintainability_signals() {
    let analysis = analyze_fixture("sloppy.ts");
    let rules = analysis
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();

    assert_eq!(analysis.parse_errors, 0);
    for expected in [
        "long-function",
        "complex-function",
        "deep-nesting",
        "parameter-bundle",
        "nested-ternary",
        "any-cluster",
        "vague-names",
        "wrapper-cluster",
    ] {
        assert!(
            rules.contains(&expected),
            "missing rule {expected:?}: {rules:?}"
        );
    }
}

#[test]
fn score_separates_clean_and_sloppy_code() {
    let clean = build_report(
        Path::new("tests/fixtures"),
        vec![analyze_fixture("clean.ts")],
        Duration::ZERO,
    );
    let sloppy = build_report(
        Path::new("tests/fixtures"),
        vec![analyze_fixture("sloppy.ts")],
        Duration::ZERO,
    );

    assert_eq!(clean.score, 0);
    assert!(
        sloppy.score >= 50,
        "unexpected sloppy score: {}",
        sloppy.score
    );
}
