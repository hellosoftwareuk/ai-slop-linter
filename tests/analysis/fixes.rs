use std::fs;

use slop::fixer;

use super::*;

#[test]
fn typescript_safe_fix_findings_are_ast_backed_and_explicitly_fixable() {
    let source = r#"
let stable = 1;
let mutable = 1;
mutable += 1;
const customer = getCustomer();
const payload = { customer: customer };
const enabled = isReady() ? true : false;
type Value = string | number | string;
type Combined = Named & Named;
function run(first: boolean, second: boolean) {
  if (first) {
    if (second) {
      return payload;
    }
  }
}
"#;
    let analysis = analyze_inline("safe-fixes.ts", source);
    let rules = analysis
        .proposed_fixes
        .iter()
        .map(|candidate| candidate.rule)
        .collect::<Vec<_>>();

    assert_eq!(
        rules.iter().filter(|rule| **rule == "prefer-const").count(),
        1,
        "only the unwritten let binding should qualify: {rules:?}"
    );
    for expected in [
        "prefer-const",
        "object-property-shorthand",
        "redundant-boolean-conditional",
        "duplicate-type-member",
        "collapsible-if",
    ] {
        assert!(rules.contains(&expected), "missing {expected}: {rules:?}");
        assert!(analysis
            .findings
            .iter()
            .any(|finding| finding.rule == expected && finding.fixable));
    }
}

#[test]
fn unsafe_or_review_sensitive_near_misses_are_not_proposed() {
    let source = r#"
let first = 1, second = 2;
let uninitialized: number;
eval("first = 3");
const prototype = { __proto__: __proto__ };
const property = { customer /* keep why */: customer };
const boolean = ready ? true /* keep why */ : false;
type Value = Named | /* keep why */ Named;
function run(first: boolean, second: boolean) {
  if (first) {
    /* keep why */
    if (second) {
      work();
    }
  }
}
"#;
    let analysis = analyze_inline("near-misses.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        analysis.proposed_fixes.is_empty(),
        "unexpected candidates: {:?}",
        analysis.proposed_fixes
    );
}

#[test]
fn fixer_preserves_bom_line_endings_and_is_idempotent() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("sample.ts");
    let source = concat!(
        "\u{feff}let stable = 1;\r\n",
        "const customer = getCustomer();\r\n",
        "const payload = { customer: customer };\r\n",
        "const enabled = ready ? true : false;\r\n",
        "type Value = string | number | string;\r\n",
        "if (enabled) {\r\n",
        "  if (stable > 0) {\r\n",
        "    use(payload);\r\n",
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
    assert_eq!(summary.files_changed, 1);
    assert!(summary.applied >= 5, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.starts_with('\u{feff}'));
    assert!(!fixed.replace("\r\n", "").contains('\n'));
    assert!(fixed.contains("const stable = 1;"));
    assert!(fixed.contains("const payload = { customer };"));
    assert!(fixed.contains("const enabled = (!!(ready));"));
    assert!(fixed.contains("type Value = string | number;"));
    assert!(fixed.contains("if ((enabled) && (stable > 0))"));

    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(rescanned.proposed_fixes.is_empty());
    let second = fixer::apply(directory.path(), &[rescanned])?;
    assert_eq!(second.applied, 0);
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn declaration_files_are_never_rewritten() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("types.d.ts");
    let source = "declare let stable: number;\n";
    fs::write(&path, source).expect("fixture should be writable");
    let mut analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )
    .expect("declaration should analyze");
    analysis.proposed_fixes.push(slop::model::ProposedFix {
        rule: "prefer-const",
        start: 8,
        end: 11,
        expected: "let".to_owned(),
        replacement: "const".to_owned(),
        line: 1,
    });

    let summary = fixer::apply(directory.path(), &[analysis]).expect("skip should succeed");
    assert_eq!(summary.applied, 0);
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}
