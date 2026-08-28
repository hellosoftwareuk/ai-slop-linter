use oxc_ast::{
    ast::{
        BindingPattern, BlockStatement, Expression, FunctionBody, IfStatement, Program, Statement,
        VariableDeclarationKind, VariableDeclarator,
    },
    AstKind,
};
use oxc_ast_visit::Visit;
use oxc_semantic::{NodeId, Semantic, SymbolId};
use oxc_span::{ContentEq, GetSpan, Span};

use crate::model::ProposedFix;

use super::typescript_fix_context::FixContext;

pub(super) fn collect_structural(program: &Program<'_>, source: &str) -> Vec<ProposedFix> {
    let mut visitor = StructuralRefactorVisitor {
        context: FixContext::new(source, &program.comments),
        generator_functions: Vec::new(),
    };
    visitor.visit_program(program);
    visitor.context.into_fixes()
}

pub(super) fn collect_semantic(
    program: &Program<'_>,
    source: &str,
    semantic: &Semantic<'_>,
) -> Vec<ProposedFix> {
    if semantic.nodes().is_empty() || super::typescript_key_remap::has_direct_eval(semantic) {
        return Vec::new();
    }
    let mut context = FixContext::new(source, &program.comments);
    for (node_id, node) in semantic.nodes().iter_enumerated() {
        let AstKind::VariableDeclarator(declarator) = node.kind() else {
            continue;
        };
        record_single_use_alias(&mut context, semantic, node_id, declarator);
    }
    context.into_fixes()
}

fn record_single_use_alias(
    context: &mut FixContext<'_, '_>,
    semantic: &Semantic<'_>,
    declarator_id: NodeId,
    declarator: &VariableDeclarator<'_>,
) {
    let Some(candidate) = alias_candidate(semantic, declarator_id, declarator) else {
        return;
    };
    let Some(reference) = single_safe_alias_use(semantic, &candidate) else {
        return;
    };
    let Some(removal) = whole_line_span(context.source(), candidate.declaration) else {
        return;
    };
    if context.has_comment(removal) || context.has_comment(reference) {
        return;
    }
    let group = candidate.declaration.start;
    context.propose_grouped("single-use-local-alias", removal, String::new(), group);
    context.propose_grouped(
        "single-use-local-alias",
        reference,
        candidate.source_name,
        group,
    );
}

struct AliasCandidate {
    declaration: Span,
    alias_symbol: SymbolId,
    source_symbol: SymbolId,
    source_name: String,
}

fn alias_candidate(
    semantic: &Semantic<'_>,
    declarator_id: NodeId,
    declarator: &VariableDeclarator<'_>,
) -> Option<AliasCandidate> {
    if declarator.type_annotation.is_some() || declarator.definite {
        return None;
    }
    let BindingPattern::BindingIdentifier(alias) = &declarator.id else {
        return None;
    };
    let Some(Expression::Identifier(source)) = declarator.init.as_ref() else {
        return None;
    };
    let nodes = semantic.nodes();
    let declaration_id = nodes.parent_id(declarator_id);
    let AstKind::VariableDeclaration(declaration) = nodes.kind(declaration_id) else {
        return None;
    };
    if declaration.kind != VariableDeclarationKind::Const
        || declaration.declare
        || declaration.declarations.len() != 1
        || !nodes
            .ancestor_kinds(declarator_id)
            .any(|kind| matches!(kind, AstKind::FunctionBody(_)))
    {
        return None;
    }
    let (Some(alias_symbol), Some(source_reference)) =
        (alias.symbol_id.get(), source.reference_id.get())
    else {
        return None;
    };
    let source_symbol = semantic
        .scoping()
        .get_reference(source_reference)
        .symbol_id()?;
    if !source_binding_is_stable(semantic, source_symbol) {
        return None;
    }
    Some(AliasCandidate {
        declaration: declaration.span,
        alias_symbol,
        source_symbol,
        source_name: source.name.to_string(),
    })
}

fn single_safe_alias_use(semantic: &Semantic<'_>, candidate: &AliasCandidate) -> Option<Span> {
    let nodes = semantic.nodes();
    let mut references = semantic.symbol_references(candidate.alias_symbol);
    let reference = references.next()?;
    if references.next().is_some()
        || !reference.is_read()
        || reference.is_write()
        || !reference.is_value()
    {
        return None;
    }
    let reference_node = nodes.get_node(reference.node_id());
    let AstKind::IdentifierReference(identifier) = reference_node.kind() else {
        return None;
    };
    if identifier.span.start <= candidate.declaration.end
        || semantic
            .scoping()
            .find_binding(reference.scope_id(), candidate.source_name.as_str().into())
            != Some(candidate.source_symbol)
        || matches!(
            nodes.parent_kind(reference.node_id()),
            AstKind::ObjectProperty(property) if property.shorthand
        )
    {
        return None;
    }
    Some(identifier.span)
}

fn source_binding_is_stable(semantic: &Semantic<'_>, symbol: SymbolId) -> bool {
    if semantic.scoping().symbol_is_mutated(symbol) {
        return false;
    }
    let declaration = semantic.symbol_declaration(symbol);
    let nodes = semantic.nodes();
    if matches!(
        declaration.kind(),
        AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_)
    ) {
        return true;
    }
    if let AstKind::VariableDeclarator(declarator) = declaration.kind() {
        return matches!(
            nodes.parent_kind(declarator.node_id.get()),
            AstKind::VariableDeclaration(declaration)
                if declaration.kind == VariableDeclarationKind::Const
        ) && nodes
            .ancestor_kinds(declaration.id())
            .any(|kind| matches!(kind, AstKind::FunctionBody(_)));
    }
    let mut parameter = false;
    let mut local_const = false;
    let mut function_local = false;
    for kind in nodes.ancestor_kinds(declaration.id()) {
        match kind {
            AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_) => parameter = true,
            AstKind::VariableDeclaration(declaration) => {
                local_const = declaration.kind == VariableDeclarationKind::Const;
            }
            AstKind::FunctionBody(_) => function_local = true,
            AstKind::ImportDeclaration(_) => return false,
            _ => {}
        }
    }
    parameter || (local_const && function_local)
}

fn whole_line_span(source: &str, span: Span) -> Option<Span> {
    let start = source[..span.start as usize]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    if !source[start..span.start as usize]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        return None;
    }
    let after = &source[span.end as usize..];
    let line_end = after
        .find('\n')
        .map_or(source.len(), |index| span.end as usize + index);
    if !source[span.end as usize..line_end].trim().is_empty() {
        return None;
    }
    let end = if line_end < source.len() {
        line_end + 1
    } else {
        line_end
    };
    Some(Span::new(start as u32, end as u32))
}

struct StructuralRefactorVisitor<'source, 'ast> {
    context: FixContext<'source, 'ast>,
    generator_functions: Vec<bool>,
}

impl StructuralRefactorVisitor<'_, '_> {
    fn record_if(&mut self, statement: &IfStatement<'_>) {
        self.record_else_after_exit(statement);
        self.record_duplicate_branch(statement);
    }

    fn record_else_after_exit(&mut self, statement: &IfStatement<'_>) {
        let Some(Statement::BlockStatement(alternate)) = statement.alternate.as_ref() else {
            return;
        };
        if alternate.body.is_empty()
            || !statement_definitely_exits(&statement.consequent)
            || !can_hoist(&alternate.body)
        {
            return;
        }
        let Some(if_without_else) = self.context.slice(Span::new(
            statement.span.start,
            statement.consequent.span().end,
        )) else {
            return;
        };
        let Some(hoisted) = render_statements(
            self.context.source(),
            &alternate.body,
            line_indent(self.context.source(), statement.span.start),
        ) else {
            return;
        };
        let replacement = format!(
            "{if_without_else}{}{hoisted}",
            newline(self.context.source())
        );
        self.context
            .propose("else-after-exit", statement.span, replacement);
    }

    fn record_duplicate_branch(&mut self, statement: &IfStatement<'_>) {
        let Statement::BlockStatement(consequent) = &statement.consequent else {
            return;
        };
        let Some(Statement::IfStatement(inner)) = statement.alternate.as_ref() else {
            return;
        };
        let Statement::BlockStatement(inner_consequent) = &inner.consequent else {
            return;
        };
        if inner.alternate.is_some()
            || consequent.body.is_empty()
            || !consequent.body.content_eq(&inner_consequent.body)
            || body_contains_identifier(consequent)
        {
            return;
        }
        let (Some(outer_test), Some(inner_test), Some(body)) = (
            self.context.slice(statement.test.span()),
            self.context.slice(inner.test.span()),
            self.context.slice(consequent.span),
        ) else {
            return;
        };
        self.context.propose(
            "duplicate-branch-body",
            statement.span,
            format!("if (({outer_test}) || ({inner_test})) {body}"),
        );
    }

    fn record_function_body(&mut self, body: &FunctionBody<'_>) {
        if self.generator_functions.last() == Some(&true) || body.statements.len() < 2 {
            return;
        }
        let Some(Statement::IfStatement(statement)) = body.statements.last() else {
            return;
        };
        let Statement::BlockStatement(consequent) = &statement.consequent else {
            return;
        };
        if statement.alternate.is_some()
            || consequent.body.len() < 2
            || !can_hoist(&consequent.body)
        {
            return;
        }
        let Some(test) = inverted_test(&self.context, &statement.test) else {
            return;
        };
        let Some(hoisted) = render_statements(
            self.context.source(),
            &consequent.body,
            line_indent(self.context.source(), statement.span.start),
        ) else {
            return;
        };
        let replacement = format!(
            "if ({test}) return;{}{hoisted}",
            newline(self.context.source())
        );
        self.context
            .propose("terminal-guard-clause", statement.span, replacement);
    }
}

impl<'a> Visit<'a> for StructuralRefactorVisitor<'_, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::Function(function) => self.generator_functions.push(function.generator),
            AstKind::ArrowFunctionExpression(_) => self.generator_functions.push(false),
            AstKind::FunctionBody(body) => self.record_function_body(body),
            AstKind::IfStatement(statement) => self.record_if(statement),
            _ => {}
        }
    }

    fn leave_node(&mut self, kind: AstKind<'a>) {
        if matches!(
            kind,
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
        ) {
            self.generator_functions.pop();
        }
    }
}

fn statement_definitely_exits(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::ReturnStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ContinueStatement(_) => true,
        Statement::BlockStatement(block) => {
            block.body.last().is_some_and(statement_definitely_exits)
        }
        Statement::IfStatement(statement) => {
            statement.alternate.as_ref().is_some_and(|alternate| {
                statement_definitely_exits(&statement.consequent)
                    && statement_definitely_exits(alternate)
            })
        }
        _ => false,
    }
}

fn can_hoist(statements: &[Statement<'_>]) -> bool {
    statements.iter().all(|statement| {
        !statement.is_declaration()
            || matches!(
                statement,
                Statement::VariableDeclaration(declaration)
                    if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var
            )
    })
}

fn inverted_test(context: &FixContext<'_, '_>, test: &Expression<'_>) -> Option<String> {
    if let Expression::UnaryExpression(unary) = test {
        if unary.operator.is_not() {
            return context.slice(unary.argument.span()).map(str::to_owned);
        }
    }
    context.slice(test.span()).map(|test| format!("!({test})"))
}

fn body_contains_identifier(body: &BlockStatement<'_>) -> bool {
    let mut identifiers = IdentifierCollector::default();
    for statement in &body.body {
        identifiers.visit_statement(statement);
    }
    identifiers.found
}

#[derive(Default)]
struct IdentifierCollector {
    found: bool,
}

impl<'a> Visit<'a> for IdentifierCollector {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        if matches!(kind, AstKind::IdentifierReference(_)) {
            self.found = true;
        }
    }
}

fn render_statements(source: &str, statements: &[Statement<'_>], indent: &str) -> Option<String> {
    let mut rendered = Vec::with_capacity(statements.len());
    for statement in statements {
        rendered.push(reindent(source, statement.span(), indent)?);
    }
    Some(rendered.join(newline(source)))
}

fn reindent(source: &str, span: Span, indent: &str) -> Option<String> {
    let statement = source.get(span.start as usize..span.end as usize)?;
    let original = line_indent(source, span.start);
    let mut lines = statement.split('\n');
    let mut output = format!("{indent}{}", lines.next()?);
    for line in lines {
        output.push('\n');
        output.push_str(indent);
        output.push_str(line.strip_prefix(original).unwrap_or(line));
    }
    Some(output)
}

fn line_indent(source: &str, offset: u32) -> &str {
    let before = &source[..offset as usize];
    let start = before.rfind('\n').map_or(0, |index| index + 1);
    let indent = &source[start..offset as usize];
    if indent.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
        indent
    } else {
        ""
    }
}

fn newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}
