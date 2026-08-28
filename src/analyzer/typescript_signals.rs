use oxc_ast::ast::{
    Argument, AssignmentTarget, CallExpression, Expression, FormalParameters,
    SimpleAssignmentTarget, Statement,
};
use oxc_span::{GetSpan, Span};

const MUTATING_METHODS: &[&str] = &[
    "add",
    "clear",
    "copyWithin",
    "delete",
    "fill",
    "pop",
    "push",
    "reverse",
    "set",
    "shift",
    "sort",
    "splice",
    "unshift",
];

pub(super) fn parameter_names(parameters: &FormalParameters<'_>) -> Vec<String> {
    parameters
        .items
        .iter()
        .filter_map(|parameter| {
            parameter
                .pattern
                .get_identifier_name()
                .map(|name| name.to_string())
        })
        .collect()
}

pub(super) fn boolean_argument_count(arguments: &[Argument<'_>]) -> usize {
    arguments
        .iter()
        .filter(|argument| matches!(argument, Argument::BooleanLiteral(_)))
        .count()
}

pub(super) fn call_chain_metrics(call: &CallExpression<'_>) -> (usize, usize) {
    let callbacks = callback_count(&call.arguments);
    previous_call(&call.callee).map_or((1, callbacks), |previous| {
        let (steps, previous_callbacks) = call_chain_metrics(previous);
        (steps + 1, callbacks + previous_callbacks)
    })
}

pub(super) fn test_callback_starts(source: &str, call: &CallExpression<'_>) -> Vec<u32> {
    if !is_test_callee(source_text(source, call.callee.span())) {
        return Vec::new();
    }
    call.arguments
        .iter()
        .filter_map(|argument| match argument {
            Argument::ArrowFunctionExpression(function) => Some(function.span.start),
            Argument::FunctionExpression(function) => Some(function.span.start),
            _ => None,
        })
        .collect()
}

pub(super) fn is_assertion_call(source: &str, call: &CallExpression<'_>) -> bool {
    let callee = source_text(source, call.callee.span()).trim();
    callee == "expect"
        || callee == "assert"
        || callee.starts_with("assert.")
        || callee.contains(".should.")
}

pub(super) fn target_mutates_parameter(
    source: &str,
    target: &AssignmentTarget<'_>,
    parameters: &[String],
) -> bool {
    rooted_at_parameter(source_text(source, target.span()), parameters)
}

pub(super) fn simple_target_mutates_parameter(
    source: &str,
    target: &SimpleAssignmentTarget<'_>,
    parameters: &[String],
) -> bool {
    rooted_at_parameter(source_text(source, target.span()), parameters)
}

pub(super) fn call_mutates_parameter(
    source: &str,
    call: &CallExpression<'_>,
    parameters: &[String],
) -> bool {
    let callee = source_text(source, call.callee.span()).trim();
    MUTATING_METHODS.iter().any(|method| {
        let suffix = format!(".{method}");
        callee.strip_suffix(&suffix).is_some_and(|receiver| {
            !receiver.contains('(') && rooted_at_parameter(receiver, parameters)
        })
    })
}

pub(super) fn catch_returns_fallback(statements: &[Statement<'_>]) -> bool {
    if statements
        .iter()
        .any(|statement| matches!(statement, Statement::ThrowStatement(_)))
    {
        return false;
    }
    statements.iter().any(|statement| {
        let Statement::ReturnStatement(return_statement) = statement else {
            return false;
        };
        return_statement
            .argument
            .as_ref()
            .is_none_or(is_default_expression)
    })
}

fn callback_count(arguments: &[Argument<'_>]) -> usize {
    arguments
        .iter()
        .filter(|argument| match argument {
            Argument::ArrowFunctionExpression(function) => {
                function
                    .get_function_body()
                    .is_some_and(|body| body.statements.len() >= 2)
                    || function.get_expression().is_some_and(|expression| {
                        matches!(
                            expression,
                            Expression::ConditionalExpression(_)
                                | Expression::LogicalExpression(_)
                                | Expression::SequenceExpression(_)
                        )
                    })
            }
            Argument::FunctionExpression(function) => function
                .body
                .as_ref()
                .is_some_and(|body| body.statements.len() >= 2),
            _ => false,
        })
        .count()
}

fn previous_call<'borrow, 'ast>(
    expression: &'borrow Expression<'ast>,
) -> Option<&'borrow CallExpression<'ast>> {
    let object = match expression {
        Expression::StaticMemberExpression(member) => &member.object,
        Expression::ComputedMemberExpression(member) => &member.object,
        Expression::PrivateFieldExpression(member) => &member.object,
        _ => return None,
    };
    match object {
        Expression::CallExpression(call) => Some(call),
        Expression::ParenthesizedExpression(parenthesized) => {
            previous_call(&parenthesized.expression)
        }
        _ => None,
    }
}

fn is_test_callee(callee: &str) -> bool {
    let callee = callee.trim();
    if callee.contains(".skip") || callee.contains(".todo") {
        return false;
    }
    matches!(callee, "it" | "test" | "specify")
        || ["it.", "test.", "specify."]
            .iter()
            .any(|prefix| callee.starts_with(prefix))
}

fn rooted_at_parameter(value: &str, parameters: &[String]) -> bool {
    let value = value.trim().trim_start_matches('(').trim_start();
    parameters.iter().any(|parameter| {
        value == parameter
            || value.starts_with(&format!("{parameter}."))
            || value.starts_with(&format!("{parameter}?."))
            || value.starts_with(&format!("{parameter}["))
    })
}

fn is_default_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ArrayExpression(_)
        | Expression::ObjectExpression(_)
        | Expression::NullLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::StringLiteral(_) => true,
        Expression::Identifier(identifier) => identifier.name == "undefined",
        _ => false,
    }
}

fn source_text(source: &str, span: Span) -> &str {
    source
        .get(span.start as usize..span.end as usize)
        .unwrap_or_default()
}
