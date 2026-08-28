use std::fs;

use slop::fixer;

use super::*;

const REFACTOR_RULES: [&str; 4] = [
    "else-after-exit",
    "terminal-guard-clause",
    "single-use-local-alias",
    "duplicate-branch-body",
];

#[test]
fn conservative_refactors_are_detected_and_applied_to_a_fixed_point() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("refactors.ts");
    let source = include_str!("../fixtures/refactors/positive.ts");
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert_eq!(analysis.parse_errors, 0);
    let rules = analysis
        .proposed_fixes
        .iter()
        .map(|candidate| candidate.rule)
        .collect::<Vec<_>>();
    for expected in REFACTOR_RULES {
        assert!(rules.contains(&expected), "missing {expected}: {rules:?}");
        assert!(analysis
            .findings
            .iter()
            .any(|finding| finding.rule == expected && finding.fixable));
    }

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.applied, 7, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.contains("return \"missing\";\n  }\n  audit(name);"));
    assert!(!fixed.contains("} else {"));
    assert!(fixed.contains("if (!(ready)) return;\n  execute();\n  report();"));
    assert!(!fixed.contains("currentInput"));
    assert!(fixed.contains("return format(input);"));
    assert!(fixed.contains("if ((primary) || (fallback)) {"));

    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(
        rescanned.proposed_fixes.is_empty(),
        "remaining fixes: {:?}",
        rescanned.proposed_fixes
    );
    let second = fixer::apply(directory.path(), &[rescanned])?;
    assert_eq!(second.applied, 0);
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn chained_alias_groups_apply_atomically_across_fixed_point_passes() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("alias-chain.ts");
    let source = r#"function consume(input: string): string {
  const firstAlias = input;
  const secondAlias = firstAlias;
  return format(secondAlias);
}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert_eq!(
        analysis
            .proposed_fixes
            .iter()
            .filter(|candidate| candidate.rule == "single-use-local-alias")
            .count(),
        4
    );
    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.applied, 4, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(!fixed.contains("Alias"));
    assert!(fixed.contains("return format(input);"));
    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert!(rescanned.proposed_fixes.is_empty());
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn conservative_refactors_preserve_uncertain_control_flow_and_aliases() {
    let source = include_str!("../fixtures/refactors/near-misses.ts");
    let analysis = analyze_inline("refactor-near-misses.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        analysis
            .proposed_fixes
            .iter()
            .all(|candidate| !REFACTOR_RULES.contains(&candidate.rule)),
        "unexpected candidates: {:?}",
        analysis.proposed_fixes
    );
}

#[test]
fn local_alias_refactor_is_disabled_by_direct_eval_and_comments() {
    let source = include_str!("../fixtures/refactors/alias-boundaries.ts");
    let (dynamic, documented) = source
        .split_once("function documented")
        .expect("fixture contains both boundaries");
    let documented = format!("function documented{documented}");
    for (path, source) in [("direct-eval.ts", dynamic), ("commented.ts", &documented)] {
        let analysis = analyze_inline(path, source);
        assert_eq!(analysis.parse_errors, 0);
        assert!(analysis
            .proposed_fixes
            .iter()
            .all(|candidate| candidate.rule != "single-use-local-alias"));
    }
}

#[test]
fn flow_refactors_preserve_crlf_and_are_idempotent() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("crlf.ts");
    let source = concat!(
        "function run(ready: boolean): void {\r\n",
        "  prepare();\r\n",
        "  if (ready) {\r\n",
        "    execute();\r\n",
        "    report();\r\n",
        "  }\r\n",
        "}\r\n",
    );
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.applied, 1);
    let fixed = fs::read_to_string(&path)?;
    assert!(!fixed.replace("\r\n", "").contains('\n'));
    assert!(fixed.contains("if (!(ready)) return;\r\n  execute();\r\n  report();"));
    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert!(rescanned.proposed_fixes.is_empty());
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}
