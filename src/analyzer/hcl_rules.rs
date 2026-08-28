use crate::model::{Category, Finding};

use super::core::{Facts, HclBlockMetrics};

const LARGE_BLOCK_LINES: usize = 80;
const DEEP_BLOCK_NESTING: usize = 4;
const COMPLEX_EXPRESSION: usize = 12;
const LARGE_COLLECTION: usize = 30;
const DYNAMIC_BLOCKS: usize = 3;
const LOCAL_VALUES: usize = 20;
const UNTYPED_VARIABLES: usize = 3;
const UNDOCUMENTED_INTERFACES: usize = 5;
const TERRAGRUNT_DEPENDENCIES: usize = 5;
const TERRAGRUNT_HOOKS: usize = 3;
const TERRAGRUNT_CONFIG_READS: usize = 4;
const TERRAGRUNT_INCLUDES: usize = 4;

pub(super) fn assess(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    for block in &facts.hcl_blocks {
        assess_block_size(path, block, findings);
        assess_expression(path, block, findings);
        assess_dynamic_blocks(path, block, findings);
        assess_local_values(path, block, findings);
    }
    assess_nesting(path, facts, findings);
    assess_interface(path, facts, findings);
    assess_sources(path, facts, findings);
    assess_lifecycle(path, facts, findings);
    assess_explicit_dependencies(path, facts, findings);
    assess_terragrunt(path, facts, findings);
}

fn assess_block_size(path: &str, block: &HclBlockMetrics, findings: &mut Vec<Finding>) {
    if block.depth != 1 || block.lines <= LARGE_BLOCK_LINES {
        return;
    }
    findings.push(Finding::new(
        "oversized-hcl-block",
        Category::Size,
        (4.0 + (block.lines - LARGE_BLOCK_LINES) as f64 / 20.0).min(12.0),
        (path.to_owned(), block.line),
        (
            format!("{} is too large to review as one unit", block_name(block)),
            format!(
                "{} physical lines with {} attributes and {} direct nested blocks; target is {LARGE_BLOCK_LINES} lines or fewer",
                block.lines, block.attributes, block.nested_blocks
            ),
        ),
    ));
}

fn assess_expression(path: &str, block: &HclBlockMetrics, findings: &mut Vec<Finding>) {
    if block.max_expression_complexity > COMPLEX_EXPRESSION {
        findings.push(Finding::new(
            "complex-hcl-expression",
            Category::Complexity,
            (4.0 + (block.max_expression_complexity - COMPLEX_EXPRESSION) as f64 / 2.0).min(12.0),
            (path.to_owned(), block.expression_line),
            (
                format!(
                    "{} hides control flow inside an expression",
                    block_name(block)
                ),
                format!(
                    "expression complexity {}; target is {COMPLEX_EXPRESSION} or lower",
                    block.max_expression_complexity
                ),
            ),
        ));
    }
    if block.max_collection_items >= LARGE_COLLECTION {
        findings.push(Finding::new(
            "large-hcl-collection",
            Category::Readability,
            (4.0 + (block.max_collection_items - LARGE_COLLECTION) as f64 / 10.0).min(10.0),
            (path.to_owned(), block.collection_line),
            (
                format!("{} embeds a large anonymous collection", block_name(block)),
                format!(
                    "{} collection elements; target is fewer than {LARGE_COLLECTION}",
                    block.max_collection_items
                ),
            ),
        ));
    }
}

fn assess_dynamic_blocks(path: &str, block: &HclBlockMetrics, findings: &mut Vec<Finding>) {
    if block.depth != 1 || block.dynamic_blocks < DYNAMIC_BLOCKS {
        return;
    }
    findings.push(Finding::new(
        "dynamic-block-cluster",
        Category::Complexity,
        (4.0 + (block.dynamic_blocks - DYNAMIC_BLOCKS) as f64).min(10.0),
        (path.to_owned(), block.line),
        (
            format!(
                "{} generates much of its shape dynamically",
                block_name(block)
            ),
            format!(
                "{} nested dynamic blocks; target is fewer than {DYNAMIC_BLOCKS}",
                block.dynamic_blocks
            ),
        ),
    ));
}

fn assess_local_values(path: &str, block: &HclBlockMetrics, findings: &mut Vec<Finding>) {
    if block.block_type != "locals" || block.attributes < LOCAL_VALUES {
        return;
    }
    findings.push(Finding::new(
        "local-value-cluster",
        Category::Abstraction,
        (4.0 + (block.attributes - LOCAL_VALUES) as f64 / 5.0).min(10.0),
        (path.to_owned(), block.line),
        (
            "A locals block acts as an implicit configuration program",
            format!(
                "{} local values in one block; target is fewer than {LOCAL_VALUES}",
                block.attributes
            ),
        ),
    ));
}

fn assess_nesting(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    let Some(block) = facts.hcl_blocks.iter().max_by_key(|block| block.depth) else {
        return;
    };
    if block.depth <= DEEP_BLOCK_NESTING {
        return;
    }
    findings.push(Finding::new(
        "deep-hcl-nesting",
        Category::Complexity,
        (4.0 + (block.depth - DEEP_BLOCK_NESTING) as f64 * 2.0).min(12.0),
        (path.to_owned(), block.line),
        (
            "Nested configuration forces readers to reconstruct too much context",
            format!(
                "block nesting depth {}; target is {DEEP_BLOCK_NESTING} or lower",
                block.depth
            ),
        ),
    ));
}

fn assess_interface(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.untyped_variables.len() >= UNTYPED_VARIABLES {
        let examples = examples(&facts.untyped_variables);
        findings.push(Finding::new(
            "untyped-variable-cluster",
            Category::TypeSafety,
            (4.0 + (facts.untyped_variables.len() - UNTYPED_VARIABLES) as f64).min(10.0),
            (path.to_owned(), facts.untyped_variables[0].1),
            (
                "Module inputs accept unconstrained values",
                format!(
                    "{} variables omit type constraints; examples: {examples}",
                    facts.untyped_variables.len()
                ),
            ),
        ));
    }
    if facts.undocumented_interfaces.len() >= UNDOCUMENTED_INTERFACES {
        let examples = examples(&facts.undocumented_interfaces);
        findings.push(Finding::new(
            "undocumented-interface-cluster",
            Category::Readability,
            (4.0 + (facts.undocumented_interfaces.len() - UNDOCUMENTED_INTERFACES) as f64 * 0.5)
                .min(9.0),
            (path.to_owned(), facts.undocumented_interfaces[0].1),
            (
                "Module inputs and outputs do not explain their contract",
                format!(
                    "{} variables or outputs omit descriptions; examples: {examples}",
                    facts.undocumented_interfaces.len()
                ),
            ),
        ));
    }
}

fn assess_sources(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    for (source, line) in &facts.floating_sources {
        findings.push(Finding::new(
            "floating-module-source",
            Category::Architecture,
            7.0,
            (path.to_owned(), *line),
            (
                "A remote module source can change without a configuration edit",
                format!("`{source}` has no version constraint, tag, commit, or ref"),
            ),
        ));
    }
}

fn assess_lifecycle(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    for line in &facts.broad_ignore_changes {
        findings.push(Finding::new(
            "broad-ignore-changes",
            Category::Architecture,
            8.0,
            (path.to_owned(), *line),
            (
                "Lifecycle configuration suppresses every managed drift signal",
                "`ignore_changes = all` prevents Terraform from reconciling any changed attribute",
            ),
        ));
    }
}

fn assess_explicit_dependencies(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    for (line, count) in &facts.wide_explicit_dependencies {
        findings.push(Finding::new(
            "explicit-dependency-cluster",
            Category::Architecture,
            (4.0 + (*count - 5) as f64 * 0.75).min(10.0),
            (path.to_owned(), *line),
            (
                "A block coordinates many objects through manual ordering",
                format!("{count} entries in one `depends_on` list; target is fewer than 5"),
            ),
        ));
    }
}

fn assess_terragrunt(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    assess_cluster(
        path,
        &facts.terragrunt_dependencies,
        ClusterRule {
            threshold: TERRAGRUNT_DEPENDENCIES,
            rule: "terragrunt-dependency-cluster",
            category: Category::Architecture,
            message: "Terragrunt configuration coordinates too many units directly",
            unit: "dependency blocks",
        },
        findings,
    );
    assess_cluster(
        path,
        &facts.terragrunt_hooks,
        ClusterRule {
            threshold: TERRAGRUNT_HOOKS,
            rule: "terragrunt-hook-cluster",
            category: Category::Complexity,
            message: "Imperative hooks hide execution flow around Terraform",
            unit: "before, after, or error hooks",
        },
        findings,
    );
    assess_cluster(
        path,
        &facts.terragrunt_config_reads,
        ClusterRule {
            threshold: TERRAGRUNT_CONFIG_READS,
            rule: "terragrunt-config-read-cluster",
            category: Category::Readability,
            message: "Configuration behavior is assembled through many external reads",
            unit: "read_terragrunt_config calls",
        },
        findings,
    );
    assess_cluster(
        path,
        &facts.terragrunt_includes,
        ClusterRule {
            threshold: TERRAGRUNT_INCLUDES,
            rule: "terragrunt-include-cluster",
            category: Category::Architecture,
            message:
                "Layered includes make the effective Terragrunt configuration hard to reconstruct",
            unit: "include blocks",
        },
        findings,
    );
}

struct ClusterRule {
    threshold: usize,
    rule: &'static str,
    category: Category,
    message: &'static str,
    unit: &'static str,
}

fn assess_cluster(path: &str, lines: &[usize], cluster: ClusterRule, findings: &mut Vec<Finding>) {
    if lines.len() < cluster.threshold {
        return;
    }
    findings.push(Finding::new(
        cluster.rule,
        cluster.category,
        (4.0 + (lines.len() - cluster.threshold) as f64).min(10.0),
        (path.to_owned(), lines[0]),
        (
            cluster.message,
            format!(
                "{} {}; target is fewer than {}",
                lines.len(),
                cluster.unit,
                cluster.threshold
            ),
        ),
    ));
}

fn examples(values: &[(String, usize)]) -> String {
    values
        .iter()
        .take(4)
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn block_name(block: &HclBlockMetrics) -> String {
    if block.label.is_empty() {
        format!("`{}` block", block.block_type)
    } else {
        format!("`{} {}` block", block.block_type, block.label)
    }
}
