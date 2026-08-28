use std::fs;

use slop::fixer;

use super::*;

const SECOND_BATCH_RULES: [&str; 7] = [
    "prefer-dot-property",
    "prefer-static-object-key",
    "empty-else",
    "redundant-terminal-return",
    "redundant-terminal-continue",
    "redundant-boolean-return",
    "unnecessary-empty-statement",
];

const THIRD_BATCH_RULES: [&str; 7] = [
    "redundant-type-identity",
    "duplicate-type-assertion",
    "duplicate-non-null-assertion",
    "jsx-boolean-shorthand",
    "collapsible-else-if",
    "invert-empty-if",
    "empty-finally",
];

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
fn second_safe_fix_batch_is_detected_and_applied_to_a_fixed_point() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("second-batch.ts");
    let source = r#";
declare const customer: { name: string } | undefined;
declare const records: string[];
declare const ready: boolean;
const direct = customer["name"];
const optional = customer?.["name"];
const payload = { ["customer"]: customer };

function decide(): boolean {
  if (ready) {
    return true;
  } else {
    return false;
  }
}

function finish(): void {
  use(payload);
  return;
}

for (const record of records) {
  use(record);
  continue;
}

if (ready) {
  use(direct, optional);
} else {}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    let rules = analysis
        .proposed_fixes
        .iter()
        .map(|candidate| candidate.rule)
        .collect::<Vec<_>>();
    for expected in SECOND_BATCH_RULES {
        assert!(rules.contains(&expected), "missing {expected}: {rules:?}");
        assert!(analysis
            .findings
            .iter()
            .any(|finding| finding.rule == expected && finding.fixable));
    }

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.applied, 9, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.starts_with('\n'));
    assert!(fixed.contains("customer.name"));
    assert!(fixed.contains("customer?.name"));
    assert!(fixed.contains("const payload = { customer };"));
    assert!(fixed.contains("return (!!(ready));"));
    assert!(!fixed.contains("return;"));
    assert!(!fixed.contains("continue;"));
    assert!(!fixed.contains("else {}"));

    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(rescanned.proposed_fixes.is_empty());
    let second = fixer::apply(directory.path(), &[rescanned])?;
    assert_eq!(second.applied, 0);
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn second_batch_preserves_ambiguous_or_documented_shapes() {
    let source = r#"
declare const object: Record<string, unknown>;
declare const key: string;
declare const active: boolean;
const numeric = 1["toString"];
const invalidName = object["not-valid"];
const dynamic = object[key];
const prototype = { ["__proto__"]: object };
const invalidKey = { ["not-valid"]: object };

if (active) {
  work();
} else {
  /* deliberately empty */
}

function documentedReturn(): void {
  work();
  // explicit boundary
  return;
}

outer: while (active) {
  continue outer;
}

while (active) {
  work();
  // explicit loop edge
  continue;
}

function documentedDecision(): boolean {
  if (active) {
    return true; // branch documents policy
  } else {
    return false;
  }
}

function documentedNoOp(): void {
  ; // deliberate no-op
  work();
}
"#;
    let analysis = analyze_inline("second-near-misses.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        analysis
            .proposed_fixes
            .iter()
            .all(|candidate| !SECOND_BATCH_RULES.contains(&candidate.rule)),
        "unexpected candidates: {:?}",
        analysis.proposed_fixes
    );
}

#[test]
fn third_safe_fix_batch_is_detected_and_applied_to_a_fixed_point() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("third-batch.tsx");
    let source = r#"
interface Named { id: string }
declare const value: Named;
declare const first: boolean;
declare const second: boolean;
type Plain = Named | never;
type Combined = Named & unknown;
const asserted = value as Named as Named;
const present = value!!;
const view = <Widget enabled={true} />;

if (first) {
  work();
} else {
  if (second) {
    workAgain();
  }
}

if (first) {
} else {
  use(value, view);
}

try {
  work();
} catch (error) {
  handle(error);
} finally {}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    let rules = analysis
        .proposed_fixes
        .iter()
        .map(|candidate| candidate.rule)
        .collect::<Vec<_>>();
    for expected in THIRD_BATCH_RULES {
        assert!(rules.contains(&expected), "missing {expected}: {rules:?}");
        assert!(analysis
            .findings
            .iter()
            .any(|finding| finding.rule == expected && finding.fixable));
    }

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.applied, 8, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.contains("type Plain = Named;"));
    assert!(fixed.contains("type Combined = Named;"));
    assert!(fixed.contains("const asserted = value as Named;"));
    assert!(fixed.contains("const present = value!;"));
    assert!(fixed.contains("<Widget enabled />"));
    assert!(fixed.contains("else if (second)"));
    assert!(fixed.contains("if (!(first)) {"));
    assert!(!fixed.contains("finally"));

    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(rescanned.proposed_fixes.is_empty());
    let second = fixer::apply(directory.path(), &[rescanned])?;
    assert_eq!(second.applied, 0);
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn third_batch_preserves_non_identity_or_documented_shapes() {
    let source = r#"
interface Named { id: string }
interface Other { name: string }
declare const value: Named;
declare const first: boolean;
declare const second: boolean;
type Bottom = never | never;
type Top = unknown & unknown;
type Documented = Named | /* deliberately explicit */ never;
const different = value as Named as Other;
const present = value!;
const disabled = <Widget enabled={false} />;
const documented = <Widget enabled={/* deliberate */ true} />;

if (first) {
  work();
} else {
  /* preserve explanation */
  if (second) workAgain();
}

if (first) {
} else {
  /* preserve explanation */
  work();
}

try {
  work();
} finally {}

try {
  work();
} catch (error) {
  handle(error);
} finally {
  /* deliberate cleanup boundary */
}
"#;
    let analysis = analyze_inline("third-near-misses.tsx", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        analysis
            .proposed_fixes
            .iter()
            .all(|candidate| !THIRD_BATCH_RULES.contains(&candidate.rule)),
        "unexpected candidates: {:?}",
        analysis.proposed_fixes
    );
}

#[test]
fn third_batch_converges_across_overlapping_and_legacy_assertion_shapes() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fixed-point.ts");
    let source = r#"
interface Named { id: string }
declare const value: Named;
type Many = never | Named | never;
const asserted = <Named><Named>value;
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert_eq!(analysis.parse_errors, 0);
    assert!(analysis
        .proposed_fixes
        .iter()
        .any(|candidate| candidate.rule == "redundant-type-identity"));
    assert!(analysis
        .proposed_fixes
        .iter()
        .any(|candidate| candidate.rule == "duplicate-type-assertion"));

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.applied, 3, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.contains("type Many = Named;"));
    assert!(fixed.contains("const asserted = <Named>value;"));
    let fixed_bytes = fixed.len() as u64;
    let rescanned = analyze_file(&path, directory.path(), fixed, fixed_bytes)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(rescanned.proposed_fixes.is_empty());
    Ok(())
}

#[test]
fn empty_else_and_empty_first_branch_skip_dangling_else_positions() {
    let source = r#"
declare const outer: boolean;
declare const inner: boolean;
if (outer)
  if (inner) work();
  else {}
else fallback();
"#;
    let analysis = analyze_inline("dangling-else.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        analysis
            .proposed_fixes
            .iter()
            .all(|candidate| !matches!(candidate.rule, "empty-else" | "invert-empty-if")),
        "unsafe candidate: {:?}",
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
        group: None,
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
