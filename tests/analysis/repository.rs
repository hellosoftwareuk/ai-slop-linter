use super::*;

#[test]
fn typescript_repository_graph_finds_cycles_barrels_and_fanout() {
    let mut sources = vec![
        ("src/cycle/a.ts".to_owned(), "import './b';".to_owned()),
        ("src/cycle/b.ts".to_owned(), "import './c';".to_owned()),
        ("src/cycle/c.ts".to_owned(), "import './a';".to_owned()),
        (
            "src/barrel/a.ts".to_owned(),
            "export * from './b';".to_owned(),
        ),
        (
            "src/barrel/b.ts".to_owned(),
            "export * from './c';".to_owned(),
        ),
        (
            "src/barrel/c.ts".to_owned(),
            "export * from './implementation';".to_owned(),
        ),
        (
            "src/barrel/implementation.ts".to_owned(),
            "export const answer = 42;".to_owned(),
        ),
    ];
    let imports = (0..12)
        .map(|index| format!("import './dependencies/dep{index}';"))
        .collect::<Vec<_>>()
        .join("\n");
    sources.push(("src/fanout.ts".to_owned(), imports));
    sources.extend((0..12).map(|index| {
        (
            format!("src/dependencies/dep{index}.ts"),
            format!("export const dep{index} = {index};"),
        )
    }));

    let report = analyze_repository(sources);
    for rule in ["dependency-cycle", "barrel-maze", "module-fanout"] {
        assert!(report_has_rule(&report, rule), "missing {rule}");
    }
    assert_eq!(report.parse_errors, 0);
    assert_eq!(
        report.repository_metrics.unresolved_relative_dependencies,
        0
    );
}

#[test]
fn repository_finds_large_structural_clones_after_renaming() {
    let first = r#"
export function buildCustomerSummary(customer: Customer) {
    const normalized = normalizeCustomer(customer);
    if (!normalized.active) {
        return { id: normalized.id, entries: [], total: 0, status: "inactive" };
    }
    const entries = normalized.orders.map(order => convertOrder(order));
    const total = entries.reduce((sum, entry) => sum + entry.amount, 0);
    const status = total > 100 ? "priority" : "standard";
    auditSummary(normalized.id, entries.length, total);
    return { id: normalized.id, entries, total, status };
}
"#;
    let second = r#"
export function buildInvoiceSummary(invoice: Invoice) {
    const prepared = normalizeInvoice(invoice);
    if (!prepared.active) {
        return { id: prepared.id, entries: [], total: 0, status: "closed" };
    }
    const entries = prepared.lines.map(line => convertLine(line));
    const total = entries.reduce((amount, entry) => amount + entry.amount, 0);
    const status = total > 500 ? "large" : "normal";
    auditInvoice(prepared.id, entries.length, total);
    return { id: prepared.id, entries, total, status };
}
"#;
    let report = analyze_repository(vec![
        ("src/customers/summary.ts".to_owned(), first.to_owned()),
        ("src/invoices/summary.ts".to_owned(), second.to_owned()),
    ]);

    assert!(
        report_has_rule(&report, "structural-clone"),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn repository_finds_rust_structural_clones_after_renaming() {
    let first = r#"
pub fn build_customer_summary(customer: Customer) -> Summary {
    let normalized = normalize_customer(customer);
    if !normalized.active {
        return Summary { id: normalized.id, entries: Vec::new(), total: 0, status: "inactive" };
    }
    let entries = normalized.orders.iter().map(convert_order).collect::<Vec<_>>();
    let total = entries.iter().map(|entry| entry.amount).sum::<i32>();
    let status = if total > 100 { "priority" } else { "standard" };
    audit_summary(normalized.id, entries.len(), total);
    Summary { id: normalized.id, entries, total, status }
}
"#;
    let second = r#"
pub fn build_invoice_summary(invoice: Invoice) -> Summary {
    let prepared = normalize_invoice(invoice);
    if !prepared.active {
        return Summary { id: prepared.id, entries: Vec::new(), total: 0, status: "closed" };
    }
    let entries = prepared.lines.iter().map(convert_line).collect::<Vec<_>>();
    let total = entries.iter().map(|line| line.amount).sum::<i32>();
    let status = if total > 500 { "large" } else { "normal" };
    audit_invoice(prepared.id, entries.len(), total);
    Summary { id: prepared.id, entries, total, status }
}
"#;
    let report = analyze_repository(vec![
        ("src/customers/summary.rs".to_owned(), first.to_owned()),
        ("src/invoices/summary.rs".to_owned(), second.to_owned()),
    ]);

    assert!(
        report_has_rule(&report, "structural-clone"),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn typescript_repository_graph_finds_coupling_hubs() {
    let mut sources = Vec::new();
    let hub_imports = (0..8)
        .map(|index| format!("import './out/dep{index}';"))
        .collect::<Vec<_>>()
        .join("\n");
    sources.push(("src/hub.ts".to_owned(), hub_imports));
    sources.extend((0..8).map(|index| {
        (
            format!("src/out/dep{index}.ts"),
            format!("export const dep{index} = {index};"),
        )
    }));
    sources.extend((0..8).map(|index| {
        (
            format!("src/in/consumer{index}.ts"),
            "import '../hub';".to_owned(),
        )
    }));

    let report = analyze_repository(sources);
    assert!(report_has_rule(&report, "coupling-hub"));
}

#[test]
fn zero_config_folder_graph_finds_navigation_and_ownership_slop() {
    let mut sources = (0..25)
        .map(|index| {
            (
                format!("src/crowded/file{index}.ts"),
                format!("export const file{index} = {index};"),
            )
        })
        .collect::<Vec<_>>();
    sources.extend((0..12).map(|index| {
        (
            format!("src/wide/child{index}/file.ts"),
            format!("export const child{index} = {index};"),
        )
    }));
    sources.push((
        "src/deep/one/two/three/four/file.ts".to_owned(),
        "export const deep = true;".to_owned(),
    ));
    sources.extend([
        (
            "src/alpha/one.ts".to_owned(),
            "import '../beta/two';".to_owned(),
        ),
        (
            "src/alpha/helper.ts".to_owned(),
            "export const helper = true;".to_owned(),
        ),
        (
            "src/beta/two.ts".to_owned(),
            "import '../alpha/helper';".to_owned(),
        ),
    ]);

    let report = analyze_repository(sources);
    for rule in [
        "crowded-folder",
        "wide-folder",
        "deep-folder-chain",
        "folder-dependency-cycle",
    ] {
        assert!(report_has_rule(&report, rule), "missing {rule}");
    }
}

#[test]
fn zero_config_folder_graph_finds_misplaced_modules_and_catch_alls() {
    let imports = (0..5)
        .map(|index| format!("import '../target/dep{index}';"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut sources = vec![("src/source/worker.ts".to_owned(), imports)];
    sources.extend((0..5).map(|index| {
        (
            format!("src/target/dep{index}.ts"),
            format!("export const dep{index} = {index};"),
        )
    }));
    sources.extend((0..12).map(|index| {
        (
            format!("src/shared/helper{index}.ts"),
            format!("export const helper{index} = {index};"),
        )
    }));
    sources.extend((0..5).map(|index| {
        (
            format!("src/feature{index}/consumer.ts"),
            format!("import '../shared/helper{index}';"),
        )
    }));

    let report = analyze_repository(sources);
    assert!(report_has_rule(&report, "misplaced-module"));
    assert!(report_has_rule(&report, "catch-all-folder"));
}

#[test]
fn rust_repository_graph_finds_cycles_barrels_and_fanout() {
    let mut sources = vec![
        ("cycle/src/lib.rs".to_owned(), "mod a; mod b;".to_owned()),
        ("cycle/src/a.rs".to_owned(), "use crate::b;".to_owned()),
        ("cycle/src/b.rs".to_owned(), "use crate::a;".to_owned()),
        (
            "barrel/src/lib.rs".to_owned(),
            "pub use crate::b::*;".to_owned(),
        ),
        (
            "barrel/src/b.rs".to_owned(),
            "pub use crate::c::*;".to_owned(),
        ),
        (
            "barrel/src/c.rs".to_owned(),
            "pub use crate::implementation::*;".to_owned(),
        ),
        (
            "barrel/src/implementation.rs".to_owned(),
            "pub const ANSWER: i32 = 42;".to_owned(),
        ),
    ];
    let declarations = (0..12)
        .map(|index| format!("mod dep{index};"))
        .collect::<Vec<_>>()
        .join("\n");
    let imported_modules = (0..12)
        .map(|index| format!("dep{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    sources.push((
        "fanout/src/lib.rs".to_owned(),
        format!("{declarations}\nuse crate::{{{imported_modules}}};"),
    ));
    sources.extend((0..12).map(|index| {
        (
            format!("fanout/src/dep{index}.rs"),
            format!("pub const DEP_{index}: i32 = {index};"),
        )
    }));

    let report = analyze_repository(sources);
    for rule in ["dependency-cycle", "barrel-maze", "module-fanout"] {
        assert!(report_has_rule(&report, rule), "missing {rule}");
    }
    assert_eq!(report.parse_errors, 0);
}

#[test]
fn zero_config_folder_graph_finds_coupling_hubs() {
    let outgoing = (0..8)
        .map(|index| format!("import '../out{index}/dependency';"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut sources = vec![("src/hub/coordinator.ts".to_owned(), outgoing)];
    sources.extend((0..8).map(|index| {
        (
            format!("src/out{index}/dependency.ts"),
            format!("export const dependency{index} = {index};"),
        )
    }));
    sources.extend((0..8).map(|index| {
        (
            format!("src/in{index}/consumer.ts"),
            "import '../hub/coordinator';".to_owned(),
        )
    }));

    let report = analyze_repository(sources);
    assert!(report_has_rule(&report, "folder-coupling-hub"));
}

#[test]
fn small_cohesive_repository_has_no_architecture_or_structure_findings() {
    let report = analyze_repository(vec![
        (
            "src/orders/model.ts".to_owned(),
            "export interface Order { id: string }".to_owned(),
        ),
        (
            "src/orders/service.ts".to_owned(),
            "import { Order } from './model'; export function save(order: Order) { return order.id; }"
                .to_owned(),
        ),
        (
            "src/api/handler.ts".to_owned(),
            "import { save } from '../orders/service'; export const handle = save;".to_owned(),
        ),
    ]);

    assert!(
        report.findings.iter().all(|finding| !matches!(
            finding.category,
            slop::model::Category::Architecture | slop::model::Category::Structure
        )),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn repository_graph_finds_dependencies_that_point_toward_volatility() {
    let mut sources = vec![(
        "src/stable.ts".to_owned(),
        "import './volatile';".to_owned(),
    )];
    sources.extend((0..8).map(|index| {
        (
            format!("src/consumer{index}.ts"),
            "import './stable';".to_owned(),
        )
    }));
    let volatile_imports = (0..8)
        .map(|index| format!("import './leaf{index}';"))
        .collect::<Vec<_>>()
        .join("\n");
    sources.push(("src/volatile.ts".to_owned(), volatile_imports));
    sources.extend((0..8).map(|index| {
        (
            format!("src/leaf{index}.ts"),
            format!("export const leaf{index} = {index};"),
        )
    }));

    let report = analyze_repository(sources);
    assert!(report_has_rule(&report, "unstable-dependency"));
}

#[test]
fn zero_config_structure_finds_wrapper_directories() {
    let report = analyze_repository(vec![
        (
            "src/wrapper/index.ts".to_owned(),
            "export * from './only/implementation';".to_owned(),
        ),
        (
            "src/wrapper/only/implementation.ts".to_owned(),
            "export const implementation = true;".to_owned(),
        ),
    ]);
    assert!(report_has_rule(&report, "wrapper-directory"));
}

#[test]
fn workspace_exports_define_zero_config_package_boundaries() {
    let root = Path::new("tests/fixtures/repositories/workspace");
    let analyses = scan(
        root,
        &ScanOptions {
            include_declarations: false,
            respect_ignores: true,
            max_file_bytes: 2_000_000,
            threads: 1,
        },
    )
    .expect("workspace fixture should be scannable");
    let report = build_report(root, analyses, Duration::ZERO);

    assert!(report_has_rule(&report, "workspace-boundary-bypass"));
}
