use super::*;

#[test]
fn typescript_finds_each_flow_readability_boundary() {
    let cases = [
        (
            "boolean-soup",
            "function check(a: boolean, b: boolean, c: boolean, d: boolean, e: boolean) { return a && b && c && d && e; }",
        ),
        (
            "else-if-chain",
            "function choose(code: number) { if (code === 1) return 1; else if (code === 2) return 2; else if (code === 3) return 3; else if (code === 4) return 4; return 0; }",
        ),
        (
            "branch-fanout",
            "function choose(code: number) { switch (code) { case 1: return 1; case 2: return 2; case 3: return 3; case 4: return 4; case 5: return 5; case 6: return 6; case 7: return 7; case 8: return 8; default: return 0; } }",
        ),
        (
            "exit-point-cluster",
            "function exit(code: number) { if (code === 1) return 1; if (code === 2) return 2; if (code === 3) return 3; if (code === 4) return 4; if (code === 5) return 5; if (code === 6) return 6; if (code === 7) return 7; if (code === 8) return 8; return 0; }",
        ),
        (
            "branch-dense-function",
            "function dense(code: number) { if (code === 1) work(); if (code === 2) work(); if (code === 3) work(); if (code === 4) work(); if (code === 5) work(); if (code === 6) work(); if (code === 7) work(); if (code === 8) work(); }",
        ),
        (
            "nested-callbacks",
            "const callbacks = (values: number[]) => values.flatMap(first => values.flatMap(second => values.map(third => first + second + third)));",
        ),
    ];

    for (rule, source) in cases {
        let analysis = analyze_inline("flow.ts", source);
        assert_eq!(analysis.parse_errors, 0, "rule {rule}");
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn typescript_flow_rules_stay_quiet_below_their_boundaries() {
    let cases = [
        (
            "boolean-soup",
            "function check(a: boolean, b: boolean, c: boolean, d: boolean) { return a && b && c && d; }",
        ),
        (
            "else-if-chain",
            "function choose(code: number) { if (code === 1) return 1; else if (code === 2) return 2; else if (code === 3) return 3; return 0; }",
        ),
        (
            "branch-fanout",
            "function choose(code: number) { switch (code) { case 1: return 1; case 2: return 2; case 3: return 3; case 4: return 4; case 5: return 5; case 6: return 6; case 7: return 7; default: return 0; } }",
        ),
        (
            "exit-point-cluster",
            "function exit(code: number) { if (code === 1) return 1; if (code === 2) return 2; if (code === 3) return 3; if (code === 4) return 4; if (code === 5) return 5; if (code === 6) return 6; if (code === 7) return 7; return 0; }",
        ),
        (
            "branch-dense-function",
            "function dense(code: number) { if (code === 1) work(); if (code === 2) work(); if (code === 3) work(); if (code === 4) work(); if (code === 5) work(); if (code === 6) work(); if (code === 7) work(); }",
        ),
        (
            "nested-callbacks",
            "const callbacks = (values: number[]) => values.flatMap(first => values.map(second => first + second));",
        ),
    ];

    for (rule, source) in cases {
        let analysis = analyze_inline("boundary.ts", source);
        assert_eq!(analysis.parse_errors, 0, "rule {rule}");
        assert!(
            !has_rule(&analysis, rule),
            "unexpected {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn rust_finds_each_flow_readability_boundary() {
    let cases = [
        (
            "boolean-soup",
            "fn check(a: bool, b: bool, c: bool, d: bool, e: bool) -> bool { a && b && c && d && e }",
        ),
        (
            "else-if-chain",
            "fn choose(code: i32) -> i32 { if code == 1 { 1 } else if code == 2 { 2 } else if code == 3 { 3 } else if code == 4 { 4 } else { 0 } }",
        ),
        (
            "branch-fanout",
            "fn choose(code: i32) -> i32 { match code { 1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7, 8 => 8, _ => 0 } }",
        ),
        (
            "exit-point-cluster",
            "fn exit(code: i32) -> i32 { if code == 1 { return 1; } if code == 2 { return 2; } if code == 3 { return 3; } if code == 4 { return 4; } if code == 5 { return 5; } if code == 6 { return 6; } if code == 7 { return 7; } if code == 8 { return 8; } return 0; }",
        ),
        (
            "branch-dense-function",
            "fn dense(code: i32) { if code == 1 { work(); } if code == 2 { work(); } if code == 3 { work(); } if code == 4 { work(); } if code == 5 { work(); } if code == 6 { work(); } if code == 7 { work(); } if code == 8 { work(); } }",
        ),
        (
            "nested-callbacks",
            "fn callbacks() { consume(|| consume(|| consume(|| 1))); }",
        ),
    ];

    for (rule, source) in cases {
        let analysis = analyze_inline("flow.rs", source);
        assert_eq!(analysis.parse_errors, 0, "rule {rule}");
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn rust_flow_rules_stay_quiet_below_their_boundaries() {
    let cases = [
        (
            "boolean-soup",
            "fn check(a: bool, b: bool, c: bool, d: bool) -> bool { a && b && c && d }",
        ),
        (
            "else-if-chain",
            "fn choose(code: i32) -> i32 { if code == 1 { 1 } else if code == 2 { 2 } else if code == 3 { 3 } else { 0 } }",
        ),
        (
            "branch-fanout",
            "fn choose(code: i32) -> i32 { match code { 1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7, _ => 0 } }",
        ),
        (
            "exit-point-cluster",
            "fn exit(code: i32) -> i32 { if code == 1 { return 1; } if code == 2 { return 2; } if code == 3 { return 3; } if code == 4 { return 4; } if code == 5 { return 5; } if code == 6 { return 6; } if code == 7 { return 7; } return 0; }",
        ),
        (
            "branch-dense-function",
            "fn dense(code: i32) { if code == 1 { work(); } if code == 2 { work(); } if code == 3 { work(); } if code == 4 { work(); } if code == 5 { work(); } if code == 6 { work(); } if code == 7 { work(); } }",
        ),
        (
            "nested-callbacks",
            "fn callbacks() { consume(|| consume(|| 1)); }",
        ),
    ];

    for (rule, source) in cases {
        let analysis = analyze_inline("boundary.rs", source);
        assert_eq!(analysis.parse_errors, 0, "rule {rule}");
        assert!(
            !has_rule(&analysis, rule),
            "unexpected {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn typescript_finds_error_state_and_async_slop() {
    let source = r#"
async function muddle(a: boolean, b: boolean, c: boolean, d: boolean) {
    let total = 0;
    total = 1; total = 2; total = 3; total = 4; total = 5;
    total = 6; total = 7; total = 8; total = 9; total = 10;
    try { work(); } catch {}
    return total;
}
"#;
    let analysis = analyze_inline("state.ts", source);

    for rule in [
        "async-without-await",
        "boolean-parameter-cluster",
        "mutation-cluster",
        "empty-catch",
    ] {
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn rust_finds_error_state_and_async_slop() {
    let source = r#"
async fn muddle(a: bool, b: bool, c: bool, d: bool) {
    let mut total = 0;
    total = 1; total = 2; total = 3; total = 4; total = 5;
    total = 6; total = 7; total = 8; total = 9; total = 10;
    one().unwrap(); two().expect("two"); three().unwrap(); four().unwrap(); five().unwrap();
}
"#;
    let analysis = analyze_inline("state.rs", source);

    for rule in [
        "async-without-await",
        "boolean-parameter-cluster",
        "mutation-cluster",
        "panic-path-cluster",
    ] {
        assert!(
            has_rule(&analysis, rule),
            "missing {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn new_flow_rules_stay_quiet_below_their_boundaries() {
    let typescript = analyze_inline(
        "boundary.ts",
        "async function okay(a: boolean, b: boolean, c: boolean) { let x = 0; x=1;x=2;x=3;x=4;x=5;x=6;x=7;x=8;x=9; try { await work(); } catch (error) { report(error); } return x; }",
    );
    let rust = analyze_inline(
        "boundary.rs",
        "async fn okay(a: bool, b: bool, c: bool) { let mut x=0; x=1;x=2;x=3;x=4;x=5;x=6;x=7;x=8;x=9; one().unwrap();two().expect(\"two\");three().unwrap();four().unwrap(); work().await; }",
    );
    let documented_catch = analyze_inline(
        "documented.ts",
        "function remove() { try { storage.removeItem('key'); } catch { /* storage may be unavailable in restricted browser modes */ } }",
    );

    for rule in [
        "async-without-await",
        "boolean-parameter-cluster",
        "mutation-cluster",
        "empty-catch",
        "panic-path-cluster",
    ] {
        assert!(!has_rule(&typescript, rule), "unexpected TS {rule}");
        assert!(!has_rule(&rust, rule), "unexpected Rust {rule}");
        assert!(
            !has_rule(&documented_catch, rule),
            "unexpected documented catch {rule}"
        );
    }
}
