use super::*;

fn generic_name_messages(analysis: &slop::model::FileAnalysis) -> Vec<&str> {
    analysis
        .findings
        .iter()
        .filter(|finding| finding.rule == "generic-function-name")
        .map(|finding| finding.message.as_str())
        .collect()
}

#[test]
fn typescript_finds_substantial_generic_functions_and_methods() {
    let source = r#"
function processData(data: number[]): number {
  const positive = data.filter((value) => value > 0);
  const doubled = positive.map((value) => value * 2);
  let total = 0;
  for (const value of doubled) {
    total += value;
  }
  return total;
}

class RequestController {
  handleRequest(request: Record<string, unknown>): string {
    if (!request.user) return "missing user";
    if (!request.action) return "missing action";
    if (!request.target) return "missing target";
    if (!request.source) return "missing source";
    if (!request.version) return "missing version";
    if (!request.timestamp) return "missing timestamp";
    return "accepted";
  }
}

const worker = {
  doWork(items: number[]): number {
    const positive = items.filter((item) => item > 0);
    const doubled = positive.map((item) => item * 2);
    let total = 0;
    for (const item of doubled) {
      total += item;
    }
    return total;
  },
};

function processInvoiceData(data: number[]): number {
  const positive = data.filter((value) => value > 0);
  const doubled = positive.map((value) => value * 2);
  let total = 0;
  for (const value of doubled) {
    total += value;
  }
  return total;
}

const adapter = {
  handleRequest(request: Request): Request {
    return request;
  },
};
"#;
    let analysis = analyze_inline("generic-names.ts", source);
    let messages = generic_name_messages(&analysis);

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(messages.len(), 3, "findings: {:?}", analysis.findings);
    let first = analysis
        .findings
        .iter()
        .find(|finding| finding.rule == "generic-function-name")
        .expect("a generic-name finding should exist");
    assert!(!first.fixable);
    assert!(first.points > 0.0);
    assert!(first.remediation_prompt.contains("domain outcome"));
    for expected in ["`processData`", "`handleRequest`", "`doWork`"] {
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "missing {expected}: {messages:?}"
        );
    }
    assert!(messages
        .iter()
        .all(|message| !message.contains("processInvoiceData")));
}

#[test]
fn rust_uses_the_same_context_sensitive_generic_name_rule() {
    let source = r#"
fn process_data(data: &[i32]) -> i32 {
    let positive = data.iter().filter(|value| **value > 0);
    let doubled = positive.map(|value| value * 2);
    let mut total = 0;
    for value in doubled {
        total += value;
    }
    total
}

struct RequestController;
impl RequestController {
    fn handle_request(&self, request: &Request) -> &'static str {
        if request.user.is_none() { return "missing user"; }
        if request.action.is_none() { return "missing action"; }
        if request.target.is_none() { return "missing target"; }
        if request.source.is_none() { return "missing source"; }
        if request.version.is_none() { return "missing version"; }
        if request.timestamp.is_none() { return "missing timestamp"; }
        "accepted"
    }
}

struct Worker;
impl Worker {
    fn do_work(&self, items: &[i32]) -> i32 {
        let positive = items.iter().filter(|item| **item > 0);
        let doubled = positive.map(|item| item * 2);
        let mut total = 0;
        for item in doubled {
            total += item;
        }
        total
    }
}

fn process_invoice_data(data: &[i32]) -> i32 {
    let positive = data.iter().filter(|value| **value > 0);
    let doubled = positive.map(|value| value * 2);
    let mut total = 0;
    for value in doubled {
        total += value;
    }
    total
}

struct Tiny;
impl Tiny {
    fn handle_request(&self, request: Request) -> Request {
        request
    }
}
"#;
    let analysis = analyze_inline("generic_names.rs", source);
    let messages = generic_name_messages(&analysis);

    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(messages.len(), 3, "findings: {:?}", analysis.findings);
    for expected in ["`process_data`", "`handle_request`", "`do_work`"] {
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "missing {expected}: {messages:?}"
        );
    }
    assert!(messages
        .iter()
        .all(|message| !message.contains("process_invoice_data")));
}

#[test]
fn framework_sized_adapters_tests_and_domain_names_stay_quiet() {
    let typescript = analyze_inline(
        "explicit-names.ts",
        r#"
function handleRequest(request: Request): Request {
  return request;
}

function calculateInvoiceTotal(lines: number[]): number {
  return lines.reduce((total, line) => total + line, 0);
}

test("processes data", () => {
  function processData(data: number[]): number {
    const positive = data.filter((value) => value > 0);
    const doubled = positive.map((value) => value * 2);
    const selected = doubled.filter((value) => value < 100);
    let total = 0;
    for (const value of selected) total += value;
    return total;
  }
  expect(processData([1])).toBe(2);
});
"#,
    );
    let rust = analyze_inline(
        "explicit_names.rs",
        r#"
fn handle_request(request: Request) -> Request {
    request
}

fn calculate_invoice_total(lines: &[i32]) -> i32 {
    lines.iter().sum()
}

#[test]
fn process_data() {
    let values = [1, 2, 3];
    let positive = values.iter().filter(|value| **value > 0);
    let doubled = positive.map(|value| value * 2);
    let total: i32 = doubled.sum();
    assert_eq!(total, 12);
}
"#,
    );

    assert!(generic_name_messages(&typescript).is_empty());
    assert!(generic_name_messages(&rust).is_empty());
}
