use std::fs;

use slop::fixer;

use super::*;

#[test]
fn closed_nested_remaps_follow_aliases_containment_and_static_accesses() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("closed-remap.ts");
    let source = r#"
function render(actionId: string): void {
  const payload = { actions: { action_id: actionId } };
  const alias = payload;
  const envelope = { payload: alias };
  use(envelope.payload.actions.action_id);
  use(envelope["payload"].actions["action_id"]);
}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert_eq!(analysis.parse_errors, 0);
    assert!(analysis.proposed_fixes.iter().any(|candidate| {
        candidate.rule == "redundant-local-key-remap"
            && candidate.expected.contains("action_id: actionId")
    }));
    assert!(
        !has_rule(&analysis, "suspicious-key-remap"),
        "findings: {:?}",
        analysis.findings
    );

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.applied, 7, "summary: {summary:?}");
    let fixed = fs::read_to_string(&path)?;
    assert!(fixed.contains("{ actions: { actionId } }"));
    assert!(!fixed.contains("const alias"));
    assert!(fixed.contains("const envelope = { payload };"));
    assert!(fixed.contains("envelope.payload.actions.actionId"));
    assert_eq!(
        fixed.matches("envelope.payload.actions.actionId").count(),
        2,
        "fixed source:\n{fixed}"
    );

    let rescanned = analyze_file(&path, directory.path(), fixed.clone(), fixed.len() as u64)?;
    assert_eq!(rescanned.parse_errors, 0);
    assert!(rescanned
        .proposed_fixes
        .iter()
        .all(|candidate| candidate.rule != "redundant-local-key-remap"));
    let second = fixer::apply(directory.path(), &[rescanned])?;
    assert_eq!(second.applied, 0);
    assert_eq!(fs::read_to_string(path)?, fixed);
    Ok(())
}

#[test]
fn boundary_reflection_dynamic_and_contract_uses_are_review_only() {
    let source = r#"
declare const actionId: string;
declare const dynamicKey: string;

function apiBoundary(): void {
  const payload = { actions: { action_id: actionId } };
  fetch("/actions", { body: JSON.stringify(payload) });
}

function returned() {
  const result = { action_id: actionId };
  return result;
}

function reflected(): void {
  const payload = { actions: { action_id: actionId } };
  Object.keys(payload.actions);
}

function dynamic(): void {
  const payload = { actions: { action_id: actionId } };
  use(payload.actions[dynamicKey]);
}

function spread(): void {
  const payload = { action_id: actionId };
  use({ ...payload });
}

function typed(): void {
  const payload: Record<string, string> = { action_id: actionId };
  use(payload.action_id);
}

function collision(other: string): void {
  const payload = { action_id: actionId, actionId: other };
  use(payload.action_id);
}

function documented(): void {
  const payload = { action_id: /* wire name */ actionId };
  use(payload.action_id);
}

function direct(): void {
  send({ action_id: actionId });
}
"#;
    let analysis = analyze_inline("observable-remaps.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(analysis
        .proposed_fixes
        .iter()
        .all(|candidate| candidate.rule != "redundant-local-key-remap"));
    let finding = analysis
        .findings
        .iter()
        .find(|finding| finding.rule == "suspicious-key-remap")
        .expect("observable remaps should remain visible for review");
    assert!(!finding.fixable);
    assert_eq!(finding.points, 0.0);
    assert!(finding.evidence.contains("not auto-fixed"));
    assert!(finding.remediation_prompt.contains("intentional wire"));
}

#[test]
fn dissimilar_or_too_short_property_names_are_not_remap_candidates() {
    let source = r#"
function render(actionId: string, id: string): void {
  const unrelated = { status: actionId };
  const tiny = { ids: id };
  use(unrelated.status, tiny.ids);
}
"#;
    let analysis = analyze_inline("unrelated-mappings.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(
        !has_rule(&analysis, "suspicious-key-remap"),
        "findings: {:?}",
        analysis.findings
    );
    assert!(analysis
        .proposed_fixes
        .iter()
        .all(|candidate| candidate.rule != "redundant-local-key-remap"));
}

#[test]
fn string_keys_abbreviations_and_small_typos_can_be_proven_locally() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("similar-remaps.ts");
    let source = r#"
function render(actionId: string, externalRef: string): void {
  const first = { "action-identifier": actionId };
  const second = { external_reference: externalRef };
  const third = { actonId: actionId };
  use(first["action-identifier"], second.external_reference, third.actonId);
}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    let remap_edits = analysis
        .proposed_fixes
        .iter()
        .filter(|candidate| candidate.rule == "redundant-local-key-remap")
        .count();
    assert_eq!(remap_edits, 6);

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.applied, 6, "summary: {summary:?}");
    let fixed = fs::read_to_string(path)?;
    assert!(fixed.contains("const first = { actionId };"));
    assert!(fixed.contains("const second = { externalRef };"));
    assert!(fixed.contains("const third = { actionId };"));
    assert!(fixed.contains("first.actionId"));
    assert!(fixed.contains("second.externalRef"));
    assert!(fixed.contains("third.actionId"));
    Ok(())
}

#[test]
fn single_call_local_helpers_are_analyzed_across_the_parameter_boundary() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("local-helper.ts");
    let source = r#"
function render(actionId: string): void {
  function consume(action) {
    use(action.action_id);
  }
  const action = { action_id: actionId };
  consume(action);
}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert!(
        !has_rule(&analysis, "suspicious-key-remap"),
        "findings: {:?}",
        analysis.findings
    );
    assert_eq!(
        analysis
            .proposed_fixes
            .iter()
            .filter(|candidate| candidate.rule == "redundant-local-key-remap")
            .count(),
        2
    );

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.applied, 2, "summary: {summary:?}");
    let fixed = fs::read_to_string(path)?;
    assert!(fixed.contains("const action = { actionId };"));
    assert!(fixed.contains("use(action.actionId);"));
    Ok(())
}

#[test]
fn helpers_with_multiple_callers_are_not_rewritten() {
    let source = r#"
function render(actionId: string, other: { action_id: string }): void {
  function consume(action) {
    use(action.action_id);
  }
  const action = { action_id: actionId };
  consume(action);
  consume(other);
}
"#;
    let analysis = analyze_inline("shared-helper.ts", source);
    assert_eq!(analysis.parse_errors, 0);
    assert!(has_rule(&analysis, "suspicious-key-remap"));
    assert!(analysis
        .proposed_fixes
        .iter()
        .all(|candidate| candidate.rule != "redundant-local-key-remap"));
}

#[test]
fn overlapping_control_flow_fixes_wait_for_the_atomic_key_rename() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("overlap.ts");
    let source = r#"
function render(actionId: string, first: boolean, second: boolean): void {
  const action = { action_id: actionId };
  if (first) {
    if (second) {
      use(action.action_id);
    }
  }
}
"#;
    fs::write(&path, source)?;
    let analysis = analyze_file(
        &path,
        directory.path(),
        source.to_owned(),
        source.len() as u64,
    )?;
    assert!(analysis
        .proposed_fixes
        .iter()
        .any(|candidate| candidate.rule == "collapsible-if"));
    assert!(analysis
        .proposed_fixes
        .iter()
        .any(|candidate| candidate.rule == "redundant-local-key-remap"));

    let summary = fixer::apply(directory.path(), &[analysis])?;
    assert_eq!(summary.applied, 3, "summary: {summary:?}");
    let fixed = fs::read_to_string(path)?;
    assert!(fixed.contains("const action = { actionId };"));
    assert!(fixed.contains("use(action.actionId);"));
    assert!(fixed.contains("if ((first) && (second))"));
    Ok(())
}
