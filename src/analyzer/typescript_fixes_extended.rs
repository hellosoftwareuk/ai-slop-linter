use oxc_ast::{
    ast::{
        Expression, FunctionBody, IfStatement, JSXAttribute, JSXAttributeValue, JSXExpression,
        Program, Statement, TSAsExpression, TSIntersectionType, TSNonNullExpression, TSType,
        TSTypeAssertion, TSUnionType, TryStatement,
    },
    AstKind,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};

use crate::model::ProposedFix;

use super::typescript_fix_context::FixContext;

pub(super) fn collect(program: &Program<'_>, source: &str) -> Vec<ProposedFix> {
    let mut visitor = ExtendedFixVisitor {
        context: FixContext::new(source, &program.comments),
    };
    visitor.visit_program(program);
    visitor.context.into_fixes()
}

struct ExtendedFixVisitor<'source, 'ast> {
    context: FixContext<'source, 'ast>,
}

impl ExtendedFixVisitor<'_, '_> {
    fn record_union_identity(&mut self, union: &TSUnionType<'_>) {
        self.record_type_identity(union.span, &union.types, |member| {
            matches!(member, TSType::TSNeverKeyword(_))
        });
    }

    fn record_intersection_identity(&mut self, intersection: &TSIntersectionType<'_>) {
        self.record_type_identity(intersection.span, &intersection.types, |member| {
            matches!(member, TSType::TSUnknownKeyword(_))
        });
    }

    fn record_type_identity(
        &mut self,
        span: Span,
        members: &[TSType<'_>],
        is_identity: impl Fn(&TSType<'_>) -> bool,
    ) {
        if members.len() < 2
            || self.context.has_comment(span)
            || !members.iter().any(|member| !is_identity(member))
        {
            return;
        }
        let Some(index) = members.iter().position(is_identity) else {
            return;
        };
        let member = members[index].span();
        let removal = if index == 0 {
            Span::new(member.start, members[1].span().start)
        } else {
            Span::new(members[index - 1].span().end, member.end)
        };
        self.context
            .propose("redundant-type-identity", removal, String::new());
    }

    fn record_as_assertion(&mut self, outer: &TSAsExpression<'_>) {
        let Expression::TSAsExpression(inner) = &outer.expression else {
            return;
        };
        self.record_duplicate_assertion(
            outer.span,
            outer.type_annotation.span(),
            inner.span,
            inner.type_annotation.span(),
        );
    }

    fn record_type_assertion(&mut self, outer: &TSTypeAssertion<'_>) {
        let Expression::TSTypeAssertion(inner) = &outer.expression else {
            return;
        };
        self.record_duplicate_assertion(
            outer.span,
            outer.type_annotation.span(),
            inner.span,
            inner.type_annotation.span(),
        );
    }

    fn record_duplicate_assertion(
        &mut self,
        outer: Span,
        outer_type: Span,
        inner: Span,
        inner_type: Span,
    ) {
        let (Some(outer_text), Some(inner_text), Some(replacement)) = (
            self.context.slice(outer_type),
            self.context.slice(inner_type),
            self.context.slice(inner),
        ) else {
            return;
        };
        if outer_text == inner_text {
            self.context
                .propose("duplicate-type-assertion", outer, replacement.to_owned());
        }
    }

    fn record_non_null_assertion(&mut self, outer: &TSNonNullExpression<'_>) {
        let Expression::TSNonNullExpression(inner) = &outer.expression else {
            return;
        };
        let Some(replacement) = self.context.slice(inner.span) else {
            return;
        };
        self.context.propose(
            "duplicate-non-null-assertion",
            outer.span,
            replacement.to_owned(),
        );
    }

    fn record_jsx_boolean(&mut self, attribute: &JSXAttribute<'_>) {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
            return;
        };
        let JSXExpression::BooleanLiteral(value) = &container.expression else {
            return;
        };
        if value.value {
            self.context.propose(
                "jsx-boolean-shorthand",
                Span::new(attribute.name.span().end, attribute.span.end),
                String::new(),
            );
        }
    }

    fn record_collapsible_else_if(&mut self, statement: &IfStatement<'_>) {
        let Some(Statement::BlockStatement(alternate)) = statement.alternate.as_ref() else {
            return;
        };
        if alternate.body.len() != 1 {
            return;
        }
        let Statement::IfStatement(inner) = &alternate.body[0] else {
            return;
        };
        let Some(replacement) = self.context.slice(inner.span) else {
            return;
        };
        self.context.propose(
            "collapsible-else-if",
            alternate.span,
            replacement.to_owned(),
        );
    }

    fn record_direct_if(&mut self, statement: &IfStatement<'_>) {
        let Statement::BlockStatement(consequent) = &statement.consequent else {
            return;
        };
        let Some(Statement::BlockStatement(alternate)) = statement.alternate.as_ref() else {
            return;
        };
        if !consequent.body.is_empty() || alternate.body.is_empty() {
            return;
        }
        let (Some(test), Some(body)) = (
            self.context.slice(statement.test.span()),
            self.context.slice(alternate.span),
        ) else {
            return;
        };
        self.context.propose(
            "invert-empty-if",
            statement.span,
            format!("if (!({test})) {body}"),
        );
    }

    fn record_statement_list(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            if let Statement::IfStatement(statement) = statement {
                self.record_direct_if(statement);
            }
        }
    }

    fn record_empty_finally(&mut self, statement: &TryStatement<'_>) {
        let (Some(handler), Some(finalizer)) = (&statement.handler, &statement.finalizer) else {
            return;
        };
        if finalizer.body.is_empty() {
            self.context.propose(
                "empty-finally",
                Span::new(handler.span.end, statement.span.end),
                String::new(),
            );
        }
    }
}

impl<'a> Visit<'a> for ExtendedFixVisitor<'_, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.record_type_or_expression_fix(kind);
        self.record_flow_fix(kind);
        self.record_container_fix(kind);
    }
}

impl ExtendedFixVisitor<'_, '_> {
    fn record_type_or_expression_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::TSUnionType(union) => self.record_union_identity(union),
            AstKind::TSIntersectionType(intersection) => {
                self.record_intersection_identity(intersection);
            }
            AstKind::TSAsExpression(expression) => self.record_as_assertion(expression),
            AstKind::TSTypeAssertion(expression) => self.record_type_assertion(expression),
            AstKind::TSNonNullExpression(expression) => self.record_non_null_assertion(expression),
            AstKind::JSXAttribute(attribute) => self.record_jsx_boolean(attribute),
            _ => {}
        }
    }

    fn record_flow_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::IfStatement(statement) => self.record_collapsible_else_if(statement),
            AstKind::TryStatement(statement) => self.record_empty_finally(statement),
            _ => {}
        }
    }

    fn record_container_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::Program(program) => self.record_statement_list(&program.body),
            AstKind::BlockStatement(block) => self.record_statement_list(&block.body),
            AstKind::FunctionBody(body) => self.record_function_body(body),
            _ => {}
        }
    }
}

impl ExtendedFixVisitor<'_, '_> {
    fn record_function_body(&mut self, body: &FunctionBody<'_>) {
        self.record_statement_list(&body.statements);
    }
}
