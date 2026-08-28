use super::*;

#[test]
fn findings_and_parse_errors_include_copyable_llm_prompts() {
    let finding = analyze_inline("prompt.ts", "async function needless() { return 1; }")
        .findings
        .into_iter()
        .find(|finding| finding.rule == "async-without-await")
        .expect("async finding");
    assert!(finding.remediation_prompt.contains("prompt.ts:1"));
    assert!(finding.remediation_prompt.contains("async-without-await"));

    let invalid = analyze_inline("broken.rs", "fn unfinished(");
    let report = build_report(Path::new("."), vec![invalid], Duration::ZERO);
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.diagnostics[0]
        .remediation_prompt
        .contains("broken.rs"));
    assert!(report.diagnostics[0]
        .remediation_prompt
        .contains("syntax error"));
}

#[test]
fn typescript_finds_duplication_adjacent_flow_slop() {
    let source = r#"
function transform(values: number[]) {
    return values.map(value => { const adjusted = value + 1; return adjusted; }).filter(value => value > 1).flatMap(value => [value]).sort().reverse();
}
function mutate(order: { status: string }) { order.status = "processed"; }
async function fallback() { try { return await load(); } catch (error) { logger.error(error); return []; } }
test("runs", () => { executeScenario(); });
configureRenderer(true, false, true);
"#;
    let analysis = analyze_inline("new-signals.ts", source);

    for rule in [
        "tangled-chain",
        "input-mutation",
        "error-laundering",
        "assertionless-test",
        "boolean-call-soup",
    ] {
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn typescript_new_signals_stay_quiet_on_explicit_flow() {
    let source = r#"
function transform(values: number[]) {
    return values.map(value => value + 1).filter(value => value > 1).sort().reverse();
}
function calculate(order: { total: number }) { let total = order.total; total += 1; return total; }
async function propagate() { try { return await load(); } catch (error) { throw error; } }
test("runs", () => { expect(executeScenario()).toBe(true); });
configureRenderer(true, false);
"#;
    let analysis = analyze_inline("new-boundaries.ts", source);

    for rule in [
        "tangled-chain",
        "input-mutation",
        "error-laundering",
        "assertionless-test",
        "boolean-call-soup",
    ] {
        assert!(
            !has_rule(&analysis, rule),
            "unexpected {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn rust_finds_duplication_adjacent_flow_slop() {
    let source = r#"
fn transform(values: Vec<i32>) -> Vec<i32> {
    values.iter().map(|value| { let adjusted = value + 1; adjusted }).filter(|value| *value > 1).flat_map(|value| [value]).collect()
}
fn mutate(left: &mut Vec<i32>, right: &mut Vec<i32>) { left.push(1); right.clear(); left.extend([2, 3]); }
fn fallback(result: Result<Vec<i32>, Error>) -> Vec<i32> {
    match result { Ok(values) => values, Err(_) => Vec::new() }
}
#[test]
fn runs() { execute_scenario(); }
fn configure() { configure_renderer(true, false, true); }
"#;
    let analysis = analyze_inline("new-signals.rs", source);

    for rule in [
        "tangled-chain",
        "input-mutation",
        "error-laundering",
        "assertionless-test",
        "boolean-call-soup",
    ] {
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn rust_new_signals_stay_quiet_on_explicit_flow() {
    let source = r#"
fn transform(values: Vec<i32>) -> Vec<i32> {
    values.iter().map(|value| value + 1).filter(|value| *value > 1).collect()
}
fn inspect(values: &mut Vec<i32>) -> usize { values.len() }
fn accumulate(values: &mut Vec<i32>) { values.push(1); values.clear(); values.extend([2, 3]); }
fn propagate(result: Result<Vec<i32>, Error>) -> Result<Vec<i32>, Error> {
    match result { Ok(values) => Ok(values), Err(error) => Err(error) }
}
#[test]
fn runs() { assert_eq!(execute_scenario(), true); }
fn configure() { configure_renderer(true, false); }
"#;
    let analysis = analyze_inline("new-boundaries.rs", source);

    for rule in [
        "tangled-chain",
        "input-mutation",
        "error-laundering",
        "assertionless-test",
        "boolean-call-soup",
    ] {
        assert!(
            !has_rule(&analysis, rule),
            "unexpected {rule}: {:?}",
            analysis.findings
        );
    }
}
