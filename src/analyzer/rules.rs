use crate::model::{AstMetrics, Category, Finding};

use super::core::{Facts, FunctionMetrics};

const LONG_FUNCTION_LINES: usize = 80;
const COMPLEX_FUNCTION: usize = 15;
const DEEP_NESTING: usize = 4;
const MANY_PARAMETERS: usize = 5;
const BOOLEAN_SOUP_OPERATORS: usize = 4;
const BRANCH_FANOUT: usize = 9;
const ELSE_IF_CONDITIONS: usize = 4;
const EXIT_POINTS: usize = 9;
const DENSE_DECISIONS: usize = 8;
const DENSE_FUNCTION_LINES: usize = 30;
const ANONYMOUS_CALLBACK_DEPTH: usize = 3;
const BOOLEAN_PARAMETERS: usize = 4;
const MUTATION_POINTS: usize = 10;
const PANIC_PATHS: usize = 5;

pub(super) fn evaluate(path: &str, file_lines: usize, facts: &Facts) -> (AstMetrics, Vec<Finding>) {
    let thin_wrappers = facts
        .functions
        .iter()
        .filter(|function| function.thin_wrapper)
        .collect::<Vec<_>>();
    let metrics = build_metrics(facts, thin_wrappers.len());
    let mut findings = Vec::new();

    for function in &facts.functions {
        assess_function(path, function, &mut findings);
        super::behavior_rules::assess_function(path, function, &mut findings);
    }
    assess_file_size(path, file_lines, &mut findings);
    assess_type_safety(path, facts, &mut findings);
    assess_names(path, facts, &mut findings);
    assess_wrappers(path, &thin_wrappers, &mut findings);
    super::behavior_rules::assess_file(path, facts, &mut findings);
    super::hcl_rules::assess(path, facts, &mut findings);
    super::fix_rules::assess(path, facts, &mut findings);

    (metrics, findings)
}

fn build_metrics(facts: &Facts, thin_wrappers: usize) -> AstMetrics {
    AstMetrics {
        functions: facts.functions.len(),
        imports: facts.imports,
        classes: facts.classes,
        interfaces: facts.interfaces,
        type_aliases: facts.type_aliases,
        structs: facts.structs,
        enums: facts.enums,
        traits: facts.traits,
        macro_invocations: facts.macro_invocations,
        macro_definitions: facts.macro_definitions,
        macro_inputs_analyzed: facts.macro_inputs_analyzed,
        macro_inputs_unresolved: facts.macro_inputs_unresolved,
        any_keywords: facts.any_locations.len(),
        type_assertions: facts.assertion_locations.len(),
        vague_bindings: facts.vague_bindings.len(),
        thin_wrappers,
        hcl_blocks: facts.hcl_blocks.len(),
        hcl_attributes: facts.hcl_blocks.iter().map(|block| block.attributes).sum(),
        terraform_resources: facts
            .hcl_blocks
            .iter()
            .filter(|block| block.block_type == "resource")
            .count(),
        terraform_variables: facts
            .hcl_blocks
            .iter()
            .filter(|block| block.block_type == "variable")
            .count(),
        terragrunt_dependencies: facts.terragrunt_dependencies.len(),
    }
}

fn assess_function(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.lines > LONG_FUNCTION_LINES {
        let excess = function.lines - LONG_FUNCTION_LINES;
        findings.push(Finding::new(
            "long-function",
            Category::Size,
            (4.0 + excess as f64 / 20.0).min(12.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` is too long to scan comfortably", function.name),
                format!(
                    "{} physical lines; target is {LONG_FUNCTION_LINES} or fewer",
                    function.lines
                ),
            ),
        ));
    }

    if function.cognitive_complexity > COMPLEX_FUNCTION {
        let excess = function.cognitive_complexity - COMPLEX_FUNCTION;
        findings.push(Finding::new(
            "complex-function",
            Category::Complexity,
            (5.0 + excess as f64 / 3.0).min(15.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` has expensive control flow", function.name),
                format!(
                    "cognitive complexity {}; target is {COMPLEX_FUNCTION} or lower",
                    function.cognitive_complexity
                ),
            ),
        ));
    }

    if function.max_nesting > DEEP_NESTING {
        let excess = function.max_nesting - DEEP_NESTING;
        findings.push(Finding::new(
            "deep-nesting",
            Category::Complexity,
            (4.0 + excess as f64 * 3.0).min(13.0),
            (path.to_owned(), function.line),
            (
                format!(
                    "`{}` forces readers to hold too many branches",
                    function.name
                ),
                format!(
                    "maximum control-flow nesting {}; target is {DEEP_NESTING} or lower",
                    function.max_nesting
                ),
            ),
        ));
    }

    assess_function_shape(path, function, findings);
    assess_flow_readability(path, function, findings);
}

fn assess_function_shape(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.parameters > MANY_PARAMETERS {
        let excess = function.parameters - MANY_PARAMETERS;
        findings.push(Finding::new(
            "parameter-bundle",
            Category::Readability,
            (3.0 + excess as f64 * 1.5).min(10.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` has a wide call contract", function.name),
                format!(
                    "{} parameters; target is {MANY_PARAMETERS} or fewer",
                    function.parameters
                ),
            ),
        ));
    }

    if function.nested_conditionals > 0 {
        findings.push(Finding::new(
            "nested-ternary",
            Category::Readability,
            (3.0 + function.nested_conditionals as f64 * 2.0).min(9.0),
            (path.to_owned(), function.line),
            (
                format!(
                    "`{}` contains nested conditional expressions",
                    function.name
                ),
                format!(
                    "{} nested ternary expression(s)",
                    function.nested_conditionals
                ),
            ),
        ));
    }

    assess_boolean_parameters(path, function, findings);
}

fn assess_flow_readability(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    assess_boolean_soup(path, function, findings);
    assess_branch_fanout(path, function, findings);
    assess_else_if_chain(path, function, findings);
    assess_exit_points(path, function, findings);
    assess_branch_density(path, function, findings);
    assess_nested_callbacks(path, function, findings);
    assess_mutation_cluster(path, function, findings);
    assess_panic_paths(path, function, findings);
}

fn assess_boolean_parameters(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.boolean_parameters < BOOLEAN_PARAMETERS {
        return;
    }
    findings.push(Finding::new(
        "boolean-parameter-cluster",
        Category::Readability,
        (4.0 + (function.boolean_parameters - BOOLEAN_PARAMETERS) as f64).min(9.0),
        (path.to_owned(), function.line),
        (
            format!("`{}` hides modes behind boolean arguments", function.name),
            format!(
                "{} boolean parameters; target is fewer than {BOOLEAN_PARAMETERS}",
                function.boolean_parameters
            ),
        ),
    ));
}

fn assess_mutation_cluster(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.mutation_points < MUTATION_POINTS {
        return;
    }
    findings.push(Finding::new(
        "mutation-cluster",
        Category::Readability,
        (4.0 + (function.mutation_points - MUTATION_POINTS) as f64 * 0.5).min(10.0),
        (path.to_owned(), function.line),
        (
            format!("`{}` changes state in too many places", function.name),
            format!(
                "{} assignment or update expressions; target is fewer than {MUTATION_POINTS}",
                function.mutation_points
            ),
        ),
    ));
}

fn assess_panic_paths(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.panic_paths < PANIC_PATHS {
        return;
    }
    findings.push(Finding::new(
        "panic-path-cluster",
        Category::Complexity,
        (4.0 + (function.panic_paths - PANIC_PATHS) as f64 * 0.75).min(10.0),
        (path.to_owned(), function.line),
        (
            format!("`{}` has many abrupt failure paths", function.name),
            format!(
                "{} panic, unwrap, expect, todo, unimplemented, or unreachable paths; target is fewer than {PANIC_PATHS}",
                function.panic_paths
            ),
        ),
    ));
}

fn assess_boolean_soup(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.max_boolean_operators >= BOOLEAN_SOUP_OPERATORS {
        let excess = function.max_boolean_operators - BOOLEAN_SOUP_OPERATORS;
        findings.push(Finding::new(
            "boolean-soup",
            Category::Readability,
            (4.0 + excess as f64).min(10.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` contains a condition that is hard to simulate", function.name),
                format!(
                    "{} logical operators in one expression; target is fewer than {BOOLEAN_SOUP_OPERATORS}",
                    function.max_boolean_operators
                ),
            ),
        ));
    }
}

fn assess_branch_fanout(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.max_branch_fanout >= BRANCH_FANOUT {
        let excess = function.max_branch_fanout - BRANCH_FANOUT;
        findings.push(Finding::new(
            "branch-fanout",
            Category::Complexity,
            (4.5 + excess as f64 * 0.75).min(11.0),
            (path.to_owned(), function.line),
            (
                format!(
                    "`{}` makes readers choose among too many branches",
                    function.name
                ),
                format!(
                    "{} cases or match arms in one branch; target is fewer than {BRANCH_FANOUT}",
                    function.max_branch_fanout
                ),
            ),
        ));
    }
}

fn assess_else_if_chain(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.max_else_if_chain >= ELSE_IF_CONDITIONS {
        let excess = function.max_else_if_chain - ELSE_IF_CONDITIONS;
        findings.push(Finding::new(
            "else-if-chain",
            Category::Readability,
            (4.0 + excess as f64).min(10.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` contains a long serial decision chain", function.name),
                format!(
                    "{} consecutive if/else-if conditions; target is fewer than {ELSE_IF_CONDITIONS}",
                    function.max_else_if_chain
                ),
            ),
        ));
    }
}

fn assess_exit_points(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.exit_points >= EXIT_POINTS {
        let excess = function.exit_points - EXIT_POINTS;
        findings.push(Finding::new(
            "exit-point-cluster",
            Category::Complexity,
            (4.0 + excess as f64 * 0.5).min(10.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` has too many paths that stop or redirect execution", function.name),
                format!(
                    "{} explicit return, throw, break, or continue points; target is fewer than {EXIT_POINTS}",
                    function.exit_points
                ),
            ),
        ));
    }
}

fn assess_branch_density(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.decision_points >= DENSE_DECISIONS && function.lines <= DENSE_FUNCTION_LINES {
        let excess = function.decision_points - DENSE_DECISIONS;
        findings.push(Finding::new(
            "branch-dense-function",
            Category::Complexity,
            (4.0 + excess as f64 * 0.5).min(11.0),
            (path.to_owned(), function.line),
            (
                format!("`{}` compresses too many decisions into a small space", function.name),
                format!(
                    "{} decision points in {} physical lines; target is fewer than {DENSE_DECISIONS} decisions in functions of {DENSE_FUNCTION_LINES} lines or fewer",
                    function.decision_points, function.lines
                ),
            ),
        ));
    }
}

fn assess_nested_callbacks(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.anonymous_depth >= ANONYMOUS_CALLBACK_DEPTH {
        let excess = function.anonymous_depth - ANONYMOUS_CALLBACK_DEPTH;
        findings.push(Finding::new(
            "nested-callbacks",
            Category::Readability,
            (5.0 + excess as f64 * 2.0).min(11.0),
            (path.to_owned(), function.line),
            (
                "Nested anonymous functions hide the order of execution",
                format!(
                    "anonymous callback or closure depth {}; target is lower than {ANONYMOUS_CALLBACK_DEPTH}",
                    function.anonymous_depth
                ),
            ),
        ));
    }
}

fn assess_file_size(path: &str, file_lines: usize, findings: &mut Vec<Finding>) {
    if file_lines <= 500 {
        return;
    }
    findings.push(Finding::new(
        "large-file",
        Category::Size,
        (5.0 + (file_lines - 500) as f64 / 100.0).min(12.0),
        (path.to_owned(), 1),
        (
            "This file contains too many concepts",
            format!("{file_lines} non-empty lines; target is 500 or fewer"),
        ),
    ));
}

fn assess_type_safety(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.any_locations.len() >= 3 {
        findings.push(Finding::new(
            "any-cluster",
            Category::TypeSafety,
            (3.0 + (facts.any_locations.len() - 3) as f64 * 0.75).min(10.0),
            (path.to_owned(), facts.any_locations[0]),
            (
                "Repeated `any` types hide the shape of data",
                format!("{} `any` keywords in this file", facts.any_locations.len()),
            ),
        ));
    }

    if facts.assertion_locations.len() >= 10 {
        findings.push(Finding::new(
            "assertion-cluster",
            Category::TypeSafety,
            (3.0 + (facts.assertion_locations.len() - 10) as f64 * 0.4).min(9.0),
            (path.to_owned(), facts.assertion_locations[0]),
            (
                "Type assertions are carrying too much of the design",
                format!(
                    "{} casts or non-null assertions in this file",
                    facts.assertion_locations.len()
                ),
            ),
        ));
    }
}

fn assess_names(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.vague_bindings.len() < 6 {
        return;
    }
    let examples = facts
        .vague_bindings
        .iter()
        .take(5)
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    findings.push(Finding::new(
        "vague-names",
        Category::Readability,
        (2.0 + (facts.vague_bindings.len() - 6) as f64 * 0.5).min(8.0),
        (path.to_owned(), facts.vague_bindings[0].1),
        (
            "Generic names make the domain harder to recover",
            format!(
                "{} vague bindings; examples: {examples}",
                facts.vague_bindings.len()
            ),
        ),
    ));
}

fn assess_wrappers(path: &str, thin_wrappers: &[&FunctionMetrics], findings: &mut Vec<Finding>) {
    if thin_wrappers.len() < 3 {
        return;
    }
    let examples = thin_wrappers
        .iter()
        .take(4)
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    findings.push(Finding::new(
        "wrapper-cluster",
        Category::Abstraction,
        (3.0 + (thin_wrappers.len() - 3) as f64).min(10.0),
        (path.to_owned(), thin_wrappers[0].line),
        (
            "Pass-through functions add navigation without hiding complexity",
            format!(
                "{} thin wrappers; examples: {examples}",
                thin_wrappers.len()
            ),
        ),
    ));
}
