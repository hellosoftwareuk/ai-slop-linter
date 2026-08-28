use ra_ap_syntax::{
    ast::{self, AstNode, HasArgList},
    Edition, SyntaxKind, SyntaxNode,
};

pub(super) fn parse_with_best_edition(
    source: &str,
) -> (ra_ap_syntax::Parse<ast::SourceFile>, Edition) {
    let mut best_edition = Edition::CURRENT;
    let mut best = ast::SourceFile::parse(source, best_edition);
    let mut fewest_errors = best.errors().len();
    if fewest_errors == 0 {
        return (best, best_edition);
    }
    for edition in [
        Edition::Edition2021,
        Edition::Edition2018,
        Edition::Edition2015,
    ] {
        let candidate = ast::SourceFile::parse(source, edition);
        let errors = candidate.errors().len();
        if errors < fewest_errors {
            best = candidate;
            best_edition = edition;
            fewest_errors = errors;
        }
        if fewest_errors == 0 {
            break;
        }
    }
    (best, best_edition)
}

pub(super) fn has_try_modifier(node: &SyntaxNode) -> bool {
    node.children()
        .any(|child| child.kind() == SyntaxKind::TRY_BLOCK_MODIFIER)
}

pub(super) fn is_logical_expression(node: &SyntaxNode) -> bool {
    ast::BinExpr::cast(node.clone()).is_some_and(|expression| {
        matches!(
            expression.op_kind(),
            Some(ast::BinaryOp::LogicOp(ast::LogicOp::And | ast::LogicOp::Or))
        )
    })
}

pub(super) fn else_if_conditions(node: &SyntaxNode) -> usize {
    let Some(mut current) = ast::IfExpr::cast(node.clone()) else {
        return 0;
    };
    let mut conditions = 1;
    while let Some(ast::ElseBranch::IfExpr(next)) = current.else_branch() {
        conditions += 1;
        current = next;
    }
    conditions
}

pub(super) fn match_arm_count(node: &SyntaxNode) -> usize {
    ast::MatchExpr::cast(node.clone())
        .and_then(|expression| expression.match_arm_list())
        .map_or(0, |arms| arms.arms().count())
}

pub(super) fn is_thin_block(block: &ast::BlockExpr, parameter_count: usize) -> bool {
    if parameter_count == 0 {
        return false;
    }
    let Some(statements) = block.stmt_list() else {
        return false;
    };
    let mut items = statements.statements();
    let first = items.next();
    if items.next().is_some() {
        return false;
    }
    let expression = match (first, statements.tail_expr()) {
        (None, Some(expression)) => Some(expression),
        (Some(ast::Stmt::ExprStmt(statement)), None) => statement.expr(),
        _ => None,
    };
    expression.is_some_and(|expression| is_thin_expression(&expression, parameter_count))
}

fn is_thin_expression(expression: &ast::Expr, parameter_count: usize) -> bool {
    match expression {
        ast::Expr::ReturnExpr(expression) => expression
            .expr()
            .is_some_and(|inner| is_thin_expression(&inner, parameter_count)),
        ast::Expr::AwaitExpr(expression) => expression
            .expr()
            .is_some_and(|inner| is_thin_expression(&inner, parameter_count)),
        ast::Expr::ParenExpr(expression) => expression
            .expr()
            .is_some_and(|inner| is_thin_expression(&inner, parameter_count)),
        ast::Expr::CallExpr(expression) => expression
            .arg_list()
            .is_some_and(|arguments| arguments_match(arguments, parameter_count)),
        ast::Expr::MethodCallExpr(expression) => expression
            .arg_list()
            .is_some_and(|arguments| arguments_match(arguments, parameter_count)),
        _ => false,
    }
}

fn arguments_match(arguments: ast::ArgList, parameter_count: usize) -> bool {
    let values = arguments.args().collect::<Vec<_>>();
    values.len() == parameter_count
        && values
            .iter()
            .all(|argument| matches!(argument, ast::Expr::PathExpr(_)))
}

pub(super) fn strip_delimiters(text: &str) -> Option<&str> {
    let first = text.chars().next()?;
    let last = text.chars().next_back()?;
    if !matches!((first, last), ('(', ')') | ('[', ']') | ('{', '}')) {
        return None;
    }
    Some(&text[first.len_utf8()..text.len() - last.len_utf8()])
}

pub(super) fn requires_structural_parse(source: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "enum",
        "fn",
        "for",
        "if",
        "let",
        "loop",
        "macro_rules",
        "match",
        "struct",
        "trait",
        "type",
        "use",
        "while",
    ];
    let has_operator = ["&&", "||", "=>", "|"]
        .into_iter()
        .any(|operator| source.contains(operator));
    let has_keyword = source
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|word| KEYWORDS.contains(&word));
    has_operator || has_keyword
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_prefilter_selects_only_structural_syntax() {
        assert!(requires_structural_parse("fn generated() {}"));
        assert!(requires_structural_parse("if ready { work() }"));
        assert!(requires_structural_parse("item => { let value = 1; }"));
        assert!(!requires_structural_parse("\"order {order_id}\", order.id"));
        assert!(!requires_structural_parse("values, Some(Expected::Value)"));
    }

    #[test]
    fn edition_fallback_does_not_reject_legacy_identifiers() {
        let (parse, edition) = parse_with_best_edition("mod gen {}");
        assert!(parse.errors().is_empty());
        assert_eq!(edition, Edition::Edition2021);
    }
}
