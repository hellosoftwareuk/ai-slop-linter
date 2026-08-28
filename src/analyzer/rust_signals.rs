use ra_ap_syntax::{
    ast::{self, AstNode, HasArgList, HasName},
    SyntaxNode,
};

const MUTATING_METHODS: &[&str] = &[
    "append",
    "clear",
    "dedup",
    "drain",
    "extend",
    "insert",
    "pop",
    "push",
    "remove",
    "reserve",
    "retain",
    "reverse",
    "set",
    "shrink_to_fit",
    "sort",
    "sort_by",
    "swap",
    "truncate",
];

pub(super) fn is_assignment_expression(node: &SyntaxNode) -> bool {
    ast::BinExpr::cast(node.clone()).is_some_and(|expression| {
        matches!(expression.op_kind(), Some(ast::BinaryOp::Assignment { .. }))
    })
}

pub(super) fn boolean_parameter_count(parameters: impl Iterator<Item = ast::Param>) -> usize {
    parameters
        .filter(|parameter| {
            parameter
                .ty()
                .is_some_and(|ty| ty.syntax().text() == "bool")
        })
        .count()
}

pub(super) fn is_panic_method(node: &SyntaxNode) -> bool {
    ast::MethodCallExpr::cast(node.clone())
        .and_then(|call| call.name_ref())
        .is_some_and(|name| matches!(name.text(), "unwrap" | "expect"))
}

pub(super) fn is_panic_macro(node: &SyntaxNode) -> bool {
    let Some(path) = ast::MacroCall::cast(node.clone()).and_then(|call| call.path()) else {
        return false;
    };
    let text = path.syntax().text().to_string();
    matches!(
        text.rsplit("::").next(),
        Some("panic" | "todo" | "unimplemented" | "unreachable")
    )
}

pub(super) fn mutable_input_names(parameters: Option<ast::ParamList>) -> Vec<String> {
    let names = parameters
        .into_iter()
        .flat_map(|parameters| parameters.params())
        .filter(|parameter| {
            parameter.ty().is_some_and(|ty| {
                ty.syntax()
                    .text()
                    .to_string()
                    .split_whitespace()
                    .collect::<String>()
                    .starts_with("&mut")
            })
        })
        .filter_map(|parameter| {
            parameter
                .pat()?
                .syntax()
                .descendants()
                .find_map(ast::IdentPat::cast)?
                .name()
                .map(|name| name.to_string())
        })
        .collect::<Vec<_>>();
    if names.len() >= 2 {
        names
    } else {
        Vec::new()
    }
}

pub(super) fn assignment_mutates_parameter(node: &SyntaxNode, parameters: &[String]) -> bool {
    ast::BinExpr::cast(node.clone())
        .and_then(|expression| expression.lhs())
        .is_some_and(|left| rooted_at_parameter(&left.syntax().text().to_string(), parameters))
}

pub(super) fn method_mutates_parameter(node: &SyntaxNode, parameters: &[String]) -> bool {
    let Some(call) = ast::MethodCallExpr::cast(node.clone()) else {
        return false;
    };
    let Some(method) = call.name_ref().map(|name| name.text().to_owned()) else {
        return false;
    };
    if !MUTATING_METHODS.contains(&method.as_str()) {
        return false;
    }
    call.receiver().is_some_and(|receiver| {
        !matches!(
            receiver,
            ast::Expr::CallExpr(_) | ast::Expr::MethodCallExpr(_)
        ) && rooted_at_parameter(&receiver.syntax().text().to_string(), parameters)
    })
}

pub(super) fn method_chain_metrics(node: &SyntaxNode) -> (usize, usize) {
    let Some(call) = ast::MethodCallExpr::cast(node.clone()) else {
        return (0, 0);
    };
    let callbacks = call
        .arg_list()
        .map_or(0, |arguments| closure_argument_count(arguments.args()));
    let Some(ast::Expr::MethodCallExpr(previous)) = call.receiver() else {
        return (1, callbacks);
    };
    let (steps, previous_callbacks) = method_chain_metrics(previous.syntax());
    (steps + 1, callbacks + previous_callbacks)
}

pub(super) fn boolean_argument_count(arguments: Option<ast::ArgList>) -> usize {
    arguments.map_or(0, |arguments| {
        arguments
            .args()
            .filter(|argument| {
                matches!(
                    argument.syntax().text().to_string().as_str(),
                    "true" | "false"
                )
            })
            .count()
    })
}

pub(super) fn is_test_function(function: &ast::Fn) -> bool {
    let attributes = function
        .syntax()
        .children()
        .filter_map(ast::Attr::cast)
        .map(|attribute| attribute.syntax().text().to_string())
        .collect::<Vec<_>>();
    attributes.iter().any(|attribute| {
        matches!(
            compact(attribute).as_str(),
            "#[test]" | "#[tokio::test]" | "#[async_std::test]" | "#[rstest]"
        )
    }) && !attributes
        .iter()
        .any(|attribute| compact(attribute) == "#[should_panic]")
}

pub(super) fn result_return_is_assertion(function: &ast::Fn) -> bool {
    function
        .ret_type()
        .is_some_and(|return_type| return_type.syntax().text().to_string().contains("Result"))
}

pub(super) fn is_assertion_macro(node: &SyntaxNode) -> bool {
    let Some(path) = ast::MacroCall::cast(node.clone()).and_then(|call| call.path()) else {
        return false;
    };
    path.syntax()
        .text()
        .to_string()
        .rsplit("::")
        .next()
        .is_some_and(|name| name.starts_with("assert"))
}

pub(super) fn is_error_laundering_match(node: &SyntaxNode) -> bool {
    let Some(arms) =
        ast::MatchExpr::cast(node.clone()).and_then(|expression| expression.match_arm_list())
    else {
        return false;
    };
    for arm in arms.arms() {
        let Some(pattern) = arm.pat() else {
            continue;
        };
        let error_pattern = compact(&pattern.syntax().text().to_string()).starts_with("Err(");
        if error_pattern
            && arm
                .expr()
                .is_some_and(|fallback| is_default_expression(&fallback))
        {
            return true;
        }
    }
    false
}

fn closure_argument_count(arguments: impl Iterator<Item = ast::Expr>) -> usize {
    arguments
        .filter_map(|argument| match argument {
            ast::Expr::ClosureExpr(closure) => Some(closure),
            _ => None,
        })
        .filter(closure_is_complex)
        .count()
}

fn closure_is_complex(closure: &ast::ClosureExpr) -> bool {
    let Some(body) = closure.body() else {
        return false;
    };
    if body.syntax().descendants().any(is_control_node) {
        return true;
    }
    matches!(body, ast::Expr::BlockExpr(block) if block.stmt_list().is_some_and(|statements| statements.statements().count() + usize::from(statements.tail_expr().is_some()) >= 2))
}

fn is_control_node(node: SyntaxNode) -> bool {
    matches!(
        node.kind(),
        ra_ap_syntax::SyntaxKind::IF_EXPR
            | ra_ap_syntax::SyntaxKind::MATCH_EXPR
            | ra_ap_syntax::SyntaxKind::FOR_EXPR
            | ra_ap_syntax::SyntaxKind::WHILE_EXPR
            | ra_ap_syntax::SyntaxKind::LOOP_EXPR
    )
}

fn rooted_at_parameter(expression: &str, parameters: &[String]) -> bool {
    let compact_expression = compact(expression);
    let expression = compact_expression
        .trim_start_matches('(')
        .trim_start_matches('*')
        .trim_start_matches('&');
    parameters.iter().any(|parameter| {
        expression == parameter
            || expression.starts_with(&format!("{parameter}."))
            || expression.starts_with(&format!("{parameter}["))
    })
}

fn is_default_expression(expression: &ast::Expr) -> bool {
    let compact_expression = compact(&expression.syntax().text().to_string());
    let fallback = compact_expression
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .unwrap_or(&compact_expression);
    matches!(
        fallback,
        "false" | "true" | "None" | "0" | "\"\"" | "[]" | "()"
    ) || fallback.ends_with("::default()")
        || fallback.ends_with("::new()")
        || fallback == "vec![]"
}

fn compact(expression: &str) -> String {
    expression
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}
