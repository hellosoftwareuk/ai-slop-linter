use std::collections::HashSet;

use oxc_ast::{
    ast::{
        Comment, ConditionalExpression, IfStatement, ObjectProperty, Program, PropertyKey,
        PropertyKind, Statement, TSIntersectionType, TSType, TSUnionType, VariableDeclaration,
        VariableDeclarationKind,
    },
    AstKind,
};
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};

use crate::model::ProposedFix;

use super::core::LineIndex;

pub(super) fn collect(program: &Program<'_>, source: &str, scoping: &Scoping) -> Vec<ProposedFix> {
    let mut visitor = FixVisitor {
        source,
        lines: LineIndex::new(source),
        comments: &program.comments,
        scoping,
        fixes: Vec::new(),
        has_direct_eval: false,
    };
    visitor.visit_program(program);
    if visitor.has_direct_eval {
        visitor
            .fixes
            .retain(|candidate| candidate.rule != "prefer-const");
    }
    visitor.fixes
}

struct FixVisitor<'source, 'ast, 'semantic> {
    source: &'source str,
    lines: LineIndex,
    comments: &'ast [Comment],
    scoping: &'semantic Scoping,
    fixes: Vec<ProposedFix>,
    has_direct_eval: bool,
}

impl FixVisitor<'_, '_, '_> {
    fn record_variable_declaration(&mut self, declaration: &VariableDeclaration<'_>) {
        if declaration.kind != VariableDeclarationKind::Let
            || declaration.declare
            || declaration.declarations.len() != 1
        {
            return;
        }
        let declarator = &declaration.declarations[0];
        if declarator.init.is_none() {
            return;
        }
        let identifiers = declarator.id.get_binding_identifiers();
        if identifiers.is_empty()
            || identifiers
                .iter()
                .any(|identifier| self.scoping.symbol_is_mutated(identifier.symbol_id()))
        {
            return;
        }

        let keyword = Span::new(declaration.span.start, declaration.span.start + 3);
        if self.slice(keyword) == Some("let") {
            self.propose("prefer-const", keyword, "const".to_owned());
        }
    }

    fn record_object_property(&mut self, property: &ObjectProperty<'_>) {
        if property.shorthand
            || property.computed
            || property.method
            || property.kind != PropertyKind::Init
        {
            return;
        }
        let PropertyKey::StaticIdentifier(key) = &property.key else {
            return;
        };
        let oxc_ast::ast::Expression::Identifier(value) = &property.value else {
            return;
        };
        if key.name == "__proto__" || key.name != value.name {
            return;
        }
        self.propose(
            "object-property-shorthand",
            property.span,
            key.name.to_string(),
        );
    }

    fn record_boolean_conditional(&mut self, expression: &ConditionalExpression<'_>) {
        let oxc_ast::ast::Expression::BooleanLiteral(consequent) = &expression.consequent else {
            return;
        };
        let oxc_ast::ast::Expression::BooleanLiteral(alternate) = &expression.alternate else {
            return;
        };
        if consequent.value == alternate.value {
            return;
        }
        let Some(test) = self.slice(expression.test.span()) else {
            return;
        };
        let replacement = if consequent.value {
            format!("(!!({test}))")
        } else {
            format!("(!({test}))")
        };
        self.propose(
            "redundant-boolean-conditional",
            expression.span,
            replacement,
        );
    }

    fn record_union(&mut self, union: &TSUnionType<'_>) {
        self.record_duplicate_type_members("duplicate-type-member", union.span, &union.types);
    }

    fn record_intersection(&mut self, intersection: &TSIntersectionType<'_>) {
        self.record_duplicate_type_members(
            "duplicate-type-member",
            intersection.span,
            &intersection.types,
        );
    }

    fn record_duplicate_type_members(
        &mut self,
        rule: &'static str,
        span: Span,
        types: &[TSType<'_>],
    ) {
        if types.len() < 2 || self.has_comment(span) {
            return;
        }
        let Some(original) = self.slice(span) else {
            return;
        };
        let mut seen = HashSet::with_capacity(types.len());
        let mut removals = Vec::new();
        for (index, member) in types.iter().enumerate() {
            let member_span = member.span();
            let Some(text) = self.slice(member_span) else {
                return;
            };
            if !seen.insert(text) && index > 0 {
                let previous_end = types[index - 1].span().end;
                removals.push((
                    (previous_end - span.start) as usize,
                    (member_span.end - span.start) as usize,
                ));
            }
        }
        if removals.is_empty() {
            return;
        }

        let mut replacement = original.to_owned();
        for (start, end) in removals.into_iter().rev() {
            replacement.replace_range(start..end, "");
        }
        self.propose(rule, span, replacement);
    }

    fn record_collapsible_if(&mut self, outer: &IfStatement<'_>) {
        if outer.alternate.is_some() {
            return;
        }
        let Statement::BlockStatement(outer_block) = &outer.consequent else {
            return;
        };
        if outer_block.body.len() != 1 {
            return;
        }
        let Statement::IfStatement(inner) = &outer_block.body[0] else {
            return;
        };
        if inner.alternate.is_some() || !matches!(&inner.consequent, Statement::BlockStatement(_)) {
            return;
        }
        let (Some(outer_test), Some(inner_test), Some(body)) = (
            self.slice(outer.test.span()),
            self.slice(inner.test.span()),
            self.slice(inner.consequent.span()),
        ) else {
            return;
        };
        self.propose(
            "collapsible-if",
            outer.span,
            format!("if (({outer_test}) && ({inner_test})) {body}"),
        );
    }

    fn record_direct_eval(&mut self, call: &oxc_ast::ast::CallExpression<'_>) {
        self.has_direct_eval |= matches!(
            call.callee.get_inner_expression(),
            oxc_ast::ast::Expression::Identifier(identifier) if identifier.name == "eval"
        );
    }

    fn propose(&mut self, rule: &'static str, span: Span, replacement: String) {
        if span.is_empty()
            || self.has_comment(span)
            || replacement == self.slice(span).unwrap_or("")
        {
            return;
        }
        let Some(expected) = self.slice(span).map(str::to_owned) else {
            return;
        };
        self.fixes.push(ProposedFix {
            rule,
            start: span.start as usize,
            end: span.end as usize,
            expected,
            replacement,
            line: self.lines.line(span.start),
        });
    }

    fn slice(&self, span: Span) -> Option<&str> {
        self.source.get(span.start as usize..span.end as usize)
    }

    fn has_comment(&self, span: Span) -> bool {
        self.comments
            .iter()
            .any(|comment| comment.span.start < span.end && comment.span.end > span.start)
    }
}

impl<'a> Visit<'a> for FixVisitor<'_, '_, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::VariableDeclaration(declaration) => {
                self.record_variable_declaration(declaration);
            }
            AstKind::ObjectProperty(property) => self.record_object_property(property),
            AstKind::ConditionalExpression(expression) => {
                self.record_boolean_conditional(expression);
            }
            AstKind::CallExpression(call) => self.record_direct_eval(call),
            other => self.record_type_or_control_fix(other),
        }
    }
}

impl FixVisitor<'_, '_, '_> {
    fn record_type_or_control_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::TSUnionType(union) => self.record_union(union),
            AstKind::TSIntersectionType(intersection) => self.record_intersection(intersection),
            AstKind::IfStatement(statement) => self.record_collapsible_if(statement),
            _ => {}
        }
    }
}
