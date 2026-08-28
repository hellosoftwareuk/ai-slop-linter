use crate::model::{Category, Finding};

use super::core::{Facts, FunctionMetrics};

const CHAIN_STEPS: usize = 5;
const CHAIN_CALLBACKS: usize = 1;
const OPAQUE_BOOLEAN_ARGUMENTS: usize = 3;

pub(super) fn assess_function(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    assess_tangled_chain(path, function, findings);
    assess_input_mutation(path, function, findings);
    assess_assertionless_test(path, function, findings);
    assess_async_contract(path, function, findings);
}

pub(super) fn assess_file(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    assess_error_laundering(path, facts, findings);
    assess_empty_catches(path, facts, findings);
    assess_boolean_calls(path, facts, findings);
    assess_key_remaps(path, facts, findings);
}

fn assess_key_remaps(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    let Some(first) = facts.key_remaps.first() else {
        return;
    };
    findings.push(Finding::new(
        "suspicious-key-remap",
        Category::Readability,
        0.0,
        (path.to_owned(), first.line),
        (
            "Similar property and value names cross an observable boundary",
            format!(
                "{} mapping(s); first `{}: {}` was not auto-fixed because {}",
                facts.key_remaps.len(),
                first.key,
                first.value,
                first.reason
            ),
        ),
    ));
}

fn assess_async_contract(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if !function.async_function || function.await_points > 0 {
        return;
    }
    findings.push(Finding::new(
        "async-without-await",
        Category::Abstraction,
        3.5,
        (path.to_owned(), function.line),
        (
            format!("`{}` is async but never suspends", function.name),
            "async function contains no await expression",
        ),
    ));
}

fn assess_empty_catches(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.empty_catches.is_empty() {
        return;
    }
    findings.push(Finding::new(
        "empty-catch",
        Category::Complexity,
        (4.0 + (facts.empty_catches.len() - 1) as f64).min(10.0),
        (path.to_owned(), facts.empty_catches[0]),
        (
            "Empty catch blocks erase failures and hide control flow",
            format!(
                "{} empty catch block(s) in this file",
                facts.empty_catches.len()
            ),
        ),
    ));
}

fn assess_tangled_chain(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    let callback_heavy =
        function.max_chain_steps >= CHAIN_STEPS && function.max_chain_callbacks >= CHAIN_CALLBACKS;
    if !callback_heavy && function.max_chain_steps < 8 {
        return;
    }
    findings.push(Finding::new(
        "tangled-chain",
        Category::Readability,
        (4.0 + (function.max_chain_steps - CHAIN_STEPS) as f64 * 0.75).min(10.0),
        (path.to_owned(), function.line),
        (
            format!(
                "`{}` hides control flow inside a fluent chain",
                function.name
            ),
            format!(
                "{} chained calls with {} complex inline callback(s)",
                function.max_chain_steps, function.max_chain_callbacks
            ),
        ),
    ));
}

fn assess_input_mutation(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.input_mutations < function.input_mutation_threshold {
        return;
    }
    findings.push(Finding::new(
        "input-mutation",
        Category::Readability,
        (4.0 + (function.input_mutations - function.input_mutation_threshold) as f64).min(10.0),
        (path.to_owned(), function.line),
        (
            format!(
                "`{}` changes values received from its caller",
                function.name
            ),
            format!(
                "{} assignment, update, or mutating call rooted at a parameter",
                function.input_mutations
            ),
        ),
    ));
}

fn assess_assertionless_test(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if !function.test_function || function.assertions > 0 {
        return;
    }
    findings.push(Finding::new(
        "assertionless-test",
        Category::Readability,
        4.0,
        (path.to_owned(), function.line),
        (
            format!(
                "`{}` executes as a test without verifying an outcome",
                function.name
            ),
            "test callback or function contains no recognized assertion",
        ),
    ));
}

fn assess_error_laundering(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.error_laundering.is_empty() {
        return;
    }
    findings.push(Finding::new(
        "error-laundering",
        Category::Complexity,
        (4.0 + (facts.error_laundering.len() - 1) as f64).min(10.0),
        (path.to_owned(), facts.error_laundering[0]),
        (
            "Failure paths are converted into ordinary-looking default values",
            format!(
                "{} catch or explicit error branch returns a default without propagation",
                facts.error_laundering.len()
            ),
        ),
    ));
}

fn assess_boolean_calls(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    if facts.boolean_literal_calls.is_empty() {
        return;
    }
    let max_arguments = facts
        .boolean_literal_calls
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(OPAQUE_BOOLEAN_ARGUMENTS);
    findings.push(Finding::new(
        "boolean-call-soup",
        Category::Readability,
        (3.5 + (max_arguments - OPAQUE_BOOLEAN_ARGUMENTS) as f64).min(9.0),
        (path.to_owned(), facts.boolean_literal_calls[0].0),
        (
            "Boolean literals make call-site intent impossible to read",
            format!(
                "{} call(s) pass at least {OPAQUE_BOOLEAN_ARGUMENTS} boolean literals; worst call passes {max_arguments}",
                facts.boolean_literal_calls.len()
            ),
        ),
    ));
}
