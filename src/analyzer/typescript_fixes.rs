use std::collections::HashSet;

use oxc_ast::{
    ast::{
        ComputedMemberExpression, ConditionalExpression, FunctionBody, IfStatement, ObjectProperty,
        Program, PropertyKey, PropertyKind, Statement, TSIntersectionType, TSType, TSUnionType,
        VariableDeclaration, VariableDeclarationKind,
    },
    AstKind,
};
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};

use crate::model::ProposedFix;

use super::typescript_fix_context::FixContext;

pub(super) fn collect(program: &Program<'_>, source: &str, scoping: &Scoping) -> Vec<ProposedFix> {
    let mut visitor = FixVisitor {
        context: FixContext::new(source, &program.comments),
        scoping,
        has_direct_eval: false,
    };
    visitor.visit_program(program);
    if visitor.has_direct_eval {
        visitor.context.remove_rule("prefer-const");
    }
    visitor.context.into_fixes()
}

struct FixVisitor<'source, 'ast, 'semantic> {
    context: FixContext<'source, 'ast>,
    scoping: &'semantic Scoping,
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
        if self.context.slice(keyword) == Some("let") {
            self.context
                .propose("prefer-const", keyword, "const".to_owned());
        }
    }

    fn record_object_property(&mut self, property: &ObjectProperty<'_>) {
        self.record_static_object_key(property);
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
        self.context.propose(
            "object-property-shorthand",
            property.span,
            key.name.to_string(),
        );
    }

    fn record_static_object_key(&mut self, property: &ObjectProperty<'_>) {
        if !property.computed || property.method || property.kind != PropertyKind::Init {
            return;
        }
        let PropertyKey::StringLiteral(key) = &property.key else {
            return;
        };
        let name = key.value.as_str();
        if name == "__proto__" || !is_ascii_identifier_name(name) {
            return;
        }
        let Some(prefix) = self
            .context
            .slice(Span::new(property.span.start, key.span.start))
        else {
            return;
        };
        let Some(suffix) = self
            .context
            .slice(Span::new(key.span.end, property.value.span().start))
        else {
            return;
        };
        let (Some(open), Some(close)) = (prefix.rfind('['), suffix.find(']')) else {
            return;
        };
        self.context.propose(
            "prefer-static-object-key",
            Span::new(
                property.span.start + open as u32,
                key.span.end + close as u32 + 1,
            ),
            name.to_owned(),
        );
    }

    fn record_dot_property(&mut self, member: &ComputedMemberExpression<'_>) {
        let oxc_ast::ast::Expression::StringLiteral(property) = &member.expression else {
            return;
        };
        let name = property.value.as_str();
        if !is_ascii_identifier_name(name)
            || matches!(
                &member.object,
                oxc_ast::ast::Expression::NumericLiteral(_)
                    | oxc_ast::ast::Expression::BigIntLiteral(_)
            )
        {
            return;
        }
        let span = Span::new(member.object.span().end, member.span.end);
        self.context.propose(
            "prefer-dot-property",
            span,
            if member.optional {
                format!("?.{name}")
            } else {
                format!(".{name}")
            },
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
        let Some(test) = self.context.slice(expression.test.span()) else {
            return;
        };
        let replacement = if consequent.value {
            format!("(!!({test}))")
        } else {
            format!("(!({test}))")
        };
        self.context.propose(
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
        if types.len() < 2 || self.context.has_comment(span) {
            return;
        }
        let Some(original) = self.context.slice(span) else {
            return;
        };
        let mut seen = HashSet::with_capacity(types.len());
        let mut removals = Vec::new();
        for (index, member) in types.iter().enumerate() {
            let member_span = member.span();
            let Some(text) = self.context.slice(member_span) else {
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
        self.context.propose(rule, span, replacement);
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
            self.context.slice(outer.test.span()),
            self.context.slice(inner.test.span()),
            self.context.slice(inner.consequent.span()),
        ) else {
            return;
        };
        self.context.propose(
            "collapsible-if",
            outer.span,
            format!("if (({outer_test}) && ({inner_test})) {body}"),
        );
    }

    fn record_if_cleanup(&mut self, statement: &IfStatement<'_>) {
        self.record_boolean_return(statement);
        self.record_collapsible_if(statement);
    }

    fn record_empty_else(&mut self, statement: &IfStatement<'_>) {
        let Some(Statement::BlockStatement(alternate)) = statement.alternate.as_ref() else {
            return;
        };
        if !alternate.body.is_empty() {
            return;
        }
        self.context.propose(
            "empty-else",
            Span::new(statement.consequent.span().end, statement.span.end),
            String::new(),
        );
    }

    fn record_boolean_return(&mut self, statement: &IfStatement<'_>) {
        let Some(alternate) = statement.alternate.as_ref() else {
            return;
        };
        let (Some(consequent), Some(alternate)) = (
            returned_boolean(&statement.consequent),
            returned_boolean(alternate),
        ) else {
            return;
        };
        if consequent == alternate {
            return;
        }
        let Some(test) = self.context.slice(statement.test.span()) else {
            return;
        };
        self.context.propose(
            "redundant-boolean-return",
            statement.span,
            if consequent {
                format!("return (!!({test}));")
            } else {
                format!("return (!({test}));")
            },
        );
    }

    fn record_terminal_return(&mut self, body: &FunctionBody<'_>) {
        let Some(Statement::ReturnStatement(statement)) = body.statements.last() else {
            return;
        };
        if statement.argument.is_some() {
            return;
        }
        let guard_start = body
            .statements
            .iter()
            .rev()
            .nth(1)
            .map_or(body.span.start, |previous| previous.span().end);
        if self
            .context
            .has_comment(Span::new(guard_start, body.span.end))
        {
            return;
        }
        self.context
            .propose("redundant-terminal-return", statement.span, String::new());
    }

    fn record_terminal_continue(&mut self, body: &Statement<'_>) {
        let Statement::BlockStatement(block) = body else {
            return;
        };
        let Some(Statement::ContinueStatement(statement)) = block.body.last() else {
            return;
        };
        if statement.label.is_some() {
            return;
        }
        let guard_start = block
            .body
            .iter()
            .rev()
            .nth(1)
            .map_or(block.span.start, |previous| previous.span().end);
        if self
            .context
            .has_comment(Span::new(guard_start, block.span.end))
        {
            return;
        }
        self.context
            .propose("redundant-terminal-continue", statement.span, String::new());
    }

    fn record_empty_statements(&mut self, statements: &[Statement<'_>], container: Span) {
        for (index, statement) in statements.iter().enumerate() {
            let Statement::EmptyStatement(empty) = statement else {
                continue;
            };
            let guard_start = index
                .checked_sub(1)
                .map_or(container.start, |previous| statements[previous].span().end);
            let guard_end = statements
                .get(index + 1)
                .map_or(container.end, |next| next.span().start);
            if !self.context.has_comment(Span::new(guard_start, guard_end)) {
                self.context
                    .propose("unnecessary-empty-statement", empty.span, String::new());
            }
        }
    }

    fn record_statement_list(&mut self, statements: &[Statement<'_>], container: Span) {
        self.record_empty_statements(statements, container);
        for statement in statements {
            if let Statement::IfStatement(statement) = statement {
                self.record_empty_else(statement);
            }
        }
    }

    fn record_direct_eval(&mut self, call: &oxc_ast::ast::CallExpression<'_>) {
        self.has_direct_eval |= matches!(
            call.callee.get_inner_expression(),
            oxc_ast::ast::Expression::Identifier(identifier) if identifier.name == "eval"
        );
    }
}

impl<'a> Visit<'a> for FixVisitor<'_, '_, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.record_declaration_or_expression_fix(kind);
        self.record_type_or_control_fix(kind);
        self.record_loop_fix(kind);
        self.record_container_fix(kind);
    }
}

impl FixVisitor<'_, '_, '_> {
    fn record_declaration_or_expression_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::VariableDeclaration(declaration) => {
                self.record_variable_declaration(declaration);
            }
            AstKind::ObjectProperty(property) => self.record_object_property(property),
            AstKind::ConditionalExpression(expression) => {
                self.record_boolean_conditional(expression);
            }
            AstKind::ComputedMemberExpression(member) => self.record_dot_property(member),
            AstKind::CallExpression(call) => self.record_direct_eval(call),
            _ => {}
        }
    }

    fn record_type_or_control_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::TSUnionType(union) => self.record_union(union),
            AstKind::TSIntersectionType(intersection) => self.record_intersection(intersection),
            AstKind::IfStatement(statement) => self.record_if_cleanup(statement),
            _ => {}
        }
    }

    fn record_loop_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::ForStatement(statement) => self.record_terminal_continue(&statement.body),
            AstKind::ForInStatement(statement) => self.record_terminal_continue(&statement.body),
            AstKind::ForOfStatement(statement) => self.record_terminal_continue(&statement.body),
            AstKind::WhileStatement(statement) => self.record_terminal_continue(&statement.body),
            AstKind::DoWhileStatement(statement) => {
                self.record_terminal_continue(&statement.body);
            }
            _ => {}
        }
    }

    fn record_container_fix(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::Program(program) => self.record_statement_list(&program.body, program.span),
            AstKind::BlockStatement(block) => {
                self.record_statement_list(&block.body, block.span);
            }
            AstKind::FunctionBody(body) => {
                self.record_statement_list(&body.statements, body.span);
                self.record_terminal_return(body);
            }
            _ => {}
        }
    }
}

fn returned_boolean(statement: &Statement<'_>) -> Option<bool> {
    let return_statement = match statement {
        Statement::ReturnStatement(statement) => statement,
        Statement::BlockStatement(block) if block.body.len() == 1 => {
            let Statement::ReturnStatement(statement) = &block.body[0] else {
                return None;
            };
            statement
        }
        _ => return None,
    };
    let Some(oxc_ast::ast::Expression::BooleanLiteral(value)) = &return_statement.argument else {
        return None;
    };
    Some(value.value)
}

fn is_ascii_identifier_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || matches!(first, b'_' | b'$'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
}
