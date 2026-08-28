use std::{collections::HashSet, path::Path};

use anyhow::Result;
use oxc_allocator::Allocator;
use oxc_ast::{
    ast::{Argument, Expression, FormalParameters, FunctionBody, IfStatement, Statement, TSType},
    AstKind,
};
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{SourceType, Span};

use crate::model::{DependencyKind, Language, ModuleDependency};

use super::clone_detection;
use super::core::{is_vague_name, FunctionInput, LineIndex, MetricCollector, ParsedFacts};
use super::typescript_signals::{
    boolean_argument_count, call_chain_metrics, call_mutates_parameter, catch_returns_fallback,
    is_assertion_call, object_function_name, parameter_names, simple_target_mutates_parameter,
    target_mutates_parameter, test_callback_starts,
};

pub(super) fn collect(path: &Path, source: &str) -> Result<ParsedFacts> {
    let source_type = SourceType::from_path(path)
        .map_err(|_| anyhow::anyhow!("unsupported TypeScript path '{}'", path.display()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let mut visitor = TypeScriptVisitor::new(source);
    if !parsed.panicked {
        visitor.metrics.facts.top_level_statements = parsed.program.body.len();
        visitor.visit_program(&parsed.program);
        let semantic = parsed.diagnostics.is_empty().then(|| {
            SemanticBuilder::new()
                .with_build_nodes(visitor.potential_key_remap)
                .build(&parsed.program)
        });
        if let Some(semantic) = semantic.filter(|result| result.diagnostics.is_empty()) {
            let semantic = semantic.semantic;
            let mut fixes =
                super::typescript_fixes::collect(&parsed.program, source, semantic.scoping());
            fixes.extend(super::typescript_fixes_extended::collect(
                &parsed.program,
                source,
            ));
            let key_remaps =
                super::typescript_key_remap::collect(&parsed.program, source, &semantic);
            fixes.extend(key_remaps.fixes);
            visitor.metrics.facts.key_remaps = key_remaps.blocked;
            visitor.metrics.facts.proposed_fixes = fixes;
        }
    }

    Ok(ParsedFacts {
        facts: visitor.metrics.facts,
        parse_errors: parsed.diagnostics.len(),
    })
}

struct TypeScriptVisitor<'source> {
    source: &'source str,
    lines: LineIndex,
    metrics: MetricCollector,
    declarator_hints: Vec<Option<String>>,
    method_hints: Vec<Option<String>>,
    property_hints: Vec<Option<String>>,
    parameter_scopes: Vec<Vec<String>>,
    test_callback_starts: HashSet<u32>,
    potential_key_remap: bool,
}

struct FunctionStart {
    name: Option<String>,
    span: Span,
    parameters: usize,
    boolean_parameters: usize,
    async_function: bool,
    parameter_names: Vec<String>,
    test_function: bool,
}

impl<'source> TypeScriptVisitor<'source> {
    fn new(source: &'source str) -> Self {
        Self {
            source,
            lines: LineIndex::new(source),
            metrics: MetricCollector::default(),
            declarator_hints: Vec::new(),
            method_hints: Vec::new(),
            property_hints: Vec::new(),
            parameter_scopes: Vec::new(),
            test_callback_starts: HashSet::new(),
            potential_key_remap: false,
        }
    }

    fn start_function(
        &mut self,
        start: FunctionStart,
        body: Option<&FunctionBody<'_>>,
        expression_body: Option<&Expression<'_>>,
    ) {
        let start_line = self.lines.line(start.span.start);
        let end_line = self.lines.line(start.span.end);
        let has_body = body.is_some() || expression_body.is_some();
        let is_named = start.name.is_some();
        let thin_wrapper = is_named
            && end_line.saturating_sub(start_line) < 6
            && (body.is_some_and(|body| is_thin_wrapper(body, start.parameters))
                || expression_body
                    .is_some_and(|expression| is_thin_expression(expression, start.parameters)));
        if is_named && has_body {
            self.record_clone_candidate(start.span, start_line, end_line);
        }
        self.parameter_scopes.push(start.parameter_names);
        self.metrics.start_function(FunctionInput {
            name: start.name,
            start_line,
            end_line,
            parameters: start.parameters,
            boolean_parameters: start.boolean_parameters,
            async_function: start.async_function,
            test_function: start.test_function,
            input_mutation_threshold: 1,
            thin_wrapper,
            has_body,
        });
    }

    fn take_context_name(&mut self) -> Option<String> {
        self.declarator_hints
            .last_mut()
            .and_then(Option::take)
            .or_else(|| self.method_hints.last_mut().and_then(Option::take))
            .or_else(|| self.property_hints.last_mut().and_then(Option::take))
    }

    fn record_fact(&mut self, kind: AstKind<'_>) {
        self.record_flow_fact(kind);
        self.record_type_fact(kind);
        self.record_declaration_fact(kind);
        self.record_module_dependency(kind);
    }

    fn record_flow_fact(&mut self, kind: AstKind<'_>) {
        self.record_control_flow(kind);
        self.record_operation_flow(kind);
    }

    fn record_control_flow(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::IfStatement(statement) => {
                self.metrics.enter_control_flow(true, false);
                self.metrics
                    .record_else_if_chain(else_if_conditions(statement));
            }
            AstKind::DoWhileStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::ForStatement(_)
            | AstKind::ForInStatement(_)
            | AstKind::ForOfStatement(_) => self.metrics.enter_control_flow(true, false),
            AstKind::CatchClause(clause) => self.record_catch(clause),
            AstKind::SwitchStatement(statement) => {
                self.metrics.enter_control_flow(true, false);
                self.metrics.record_branch_fanout(statement.cases.len());
            }
            AstKind::ConditionalExpression(_) => self.metrics.enter_control_flow(true, true),
            _ => {}
        }
    }

    fn record_catch(&mut self, clause: &oxc_ast::ast::CatchClause<'_>) {
        self.metrics.enter_control_flow(true, false);
        if clause.body.body.is_empty() && block_body_is_blank(self.source, clause.body.span) {
            self.metrics
                .facts
                .empty_catches
                .push(self.lines.line(clause.span.start));
        }
        if catch_returns_fallback(&clause.body.body) {
            self.metrics
                .facts
                .error_laundering
                .push(self.lines.line(clause.span.start));
        }
    }

    fn record_operation_flow(&mut self, kind: AstKind<'_>) {
        self.record_expression_flow(kind);
        if matches!(
            kind,
            AstKind::ReturnStatement(_)
                | AstKind::ThrowStatement(_)
                | AstKind::BreakStatement(_)
                | AstKind::ContinueStatement(_)
        ) {
            self.metrics.record_exit_point();
        }
    }

    fn record_expression_flow(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::LogicalExpression(_) => self.metrics.enter_boolean_operator(),
            AstKind::AwaitExpression(_) => self.metrics.record_await_point(),
            AstKind::AssignmentExpression(expression) => self.record_assignment(expression),
            AstKind::UpdateExpression(expression) => self.record_update(expression),
            AstKind::CallExpression(call) => self.record_call(call),
            AstKind::NewExpression(expression) => {
                self.record_boolean_call(expression.span.start, &expression.arguments);
            }
            _ => {}
        }
    }

    fn record_assignment(&mut self, expression: &oxc_ast::ast::AssignmentExpression<'_>) {
        self.metrics.record_mutation();
        if target_mutates_parameter(self.source, &expression.left, self.current_parameters()) {
            self.metrics.record_input_mutation();
        }
    }

    fn record_update(&mut self, expression: &oxc_ast::ast::UpdateExpression<'_>) {
        self.metrics.record_mutation();
        if simple_target_mutates_parameter(
            self.source,
            &expression.argument,
            self.current_parameters(),
        ) {
            self.metrics.record_input_mutation();
        }
    }

    fn record_call(&mut self, call: &oxc_ast::ast::CallExpression<'_>) {
        self.test_callback_starts
            .extend(test_callback_starts(self.source, call));
        let (steps, callbacks) = call_chain_metrics(call);
        self.metrics.record_chain(steps, callbacks);
        self.record_boolean_call(call.span.start, &call.arguments);
        if is_assertion_call(self.source, call) {
            self.metrics.record_assertion();
        }
        if call_mutates_parameter(self.source, call, self.current_parameters()) {
            self.metrics.record_input_mutation();
        }
    }

    fn record_boolean_call(&mut self, offset: u32, arguments: &[Argument<'_>]) {
        let count = boolean_argument_count(arguments);
        if count >= 3 {
            self.metrics
                .facts
                .boolean_literal_calls
                .push((self.lines.line(offset), count));
        }
    }

    fn current_parameters(&self) -> &[String] {
        self.parameter_scopes.last().map_or(&[], Vec::as_slice)
    }

    fn record_clone_candidate(&mut self, span: Span, line: usize, end_line: usize) {
        let Some(source) = self.source.get(span.start as usize..span.end as usize) else {
            return;
        };
        if let Some(candidate) =
            clone_detection::candidate(source, Language::TypeScript, line, end_line)
        {
            self.metrics.facts.clone_candidates.push(candidate);
        }
    }

    fn record_type_fact(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::TSAnyKeyword(keyword) => self
                .metrics
                .facts
                .any_locations
                .push(self.lines.line(keyword.span.start)),
            AstKind::TSAsExpression(expression) => self.record_assertion(expression.span.start),
            AstKind::TSTypeAssertion(expression) => self.record_assertion(expression.span.start),
            AstKind::TSNonNullExpression(expression) => {
                self.record_assertion(expression.span.start);
            }
            _ => {}
        }
    }

    fn record_declaration_fact(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::BindingIdentifier(identifier) => self.record_binding_fact(identifier),
            AstKind::ImportDeclaration(_) => self.metrics.facts.imports += 1,
            AstKind::Class(_) => self.metrics.facts.classes += 1,
            AstKind::TSInterfaceDeclaration(_) => self.metrics.facts.interfaces += 1,
            AstKind::TSTypeAliasDeclaration(_) => self.metrics.facts.type_aliases += 1,
            AstKind::ObjectProperty(property) => self.record_key_remap_candidate(property),
            _ => {}
        }
    }

    fn record_binding_fact(&mut self, identifier: &oxc_ast::ast::BindingIdentifier<'_>) {
        let name = identifier.name.as_str();
        if is_vague_name(name) {
            self.metrics
                .facts
                .vague_bindings
                .push((name.to_string(), self.lines.line(identifier.span.start)));
        }
    }

    fn record_key_remap_candidate(&mut self, property: &oxc_ast::ast::ObjectProperty<'_>) {
        self.potential_key_remap |= super::typescript_key_remap::is_potential_property(property);
    }

    fn enter_object_property(&mut self, property: &oxc_ast::ast::ObjectProperty<'_>) {
        self.property_hints.push(object_function_name(property));
        self.record_fact(AstKind::ObjectProperty(property));
    }

    fn record_assertion(&mut self, offset: u32) {
        self.metrics
            .facts
            .assertion_locations
            .push(self.lines.line(offset));
    }

    fn record_module_dependency(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::ImportDeclaration(declaration) => self.push_dependency(
                declaration.source.value.as_str(),
                declaration.span.start,
                DependencyKind::Import,
            ),
            AstKind::ExportAllDeclaration(declaration) => self.push_dependency(
                declaration.source.value.as_str(),
                declaration.span.start,
                DependencyKind::ReExport,
            ),
            AstKind::ExportFromDeclaration(declaration) => self.push_dependency(
                declaration.source.value.as_str(),
                declaration.span.start,
                DependencyKind::ReExport,
            ),
            AstKind::ImportExpression(expression) => {
                if let Expression::StringLiteral(source) = &expression.source {
                    self.push_dependency(
                        source.value.as_str(),
                        expression.span.start,
                        DependencyKind::Import,
                    );
                }
            }
            _ => {}
        }
    }

    fn push_dependency(&mut self, specifier: &str, offset: u32, kind: DependencyKind) {
        self.metrics.facts.dependencies.push(ModuleDependency {
            specifier: specifier.to_owned(),
            line: self.lines.line(offset),
            kind,
        });
    }
}

impl<'a> Visit<'a> for TypeScriptVisitor<'_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::VariableDeclarator(declaration) => {
                let direct_function = declaration.init.as_ref().is_some_and(|expression| {
                    matches!(
                        expression,
                        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
                    )
                });
                self.declarator_hints.push(
                    direct_function
                        .then(|| {
                            declaration
                                .id
                                .get_identifier_name()
                                .map(|name| name.to_string())
                        })
                        .flatten(),
                );
            }
            AstKind::MethodDefinition(method) => self
                .method_hints
                .push(method.key.static_name().map(|name| name.into_owned())),
            AstKind::ObjectProperty(property) => self.enter_object_property(property),
            AstKind::Function(function) => {
                let context_name = self.take_context_name();
                let test_function = self.test_callback_starts.remove(&function.span.start);
                let name = function
                    .id
                    .as_ref()
                    .map(|identifier| identifier.name.to_string())
                    .or(context_name);
                self.start_function(
                    FunctionStart {
                        name,
                        span: function.span,
                        parameters: function.params.items.len()
                            + usize::from(function.params.rest.is_some()),
                        boolean_parameters: boolean_parameter_count(&function.params),
                        async_function: function.r#async,
                        parameter_names: parameter_names(&function.params),
                        test_function,
                    },
                    function.body.as_deref(),
                    None,
                );
            }
            AstKind::ArrowFunctionExpression(function) => {
                let name = self.take_context_name();
                let test_function = self.test_callback_starts.remove(&function.span.start);
                self.start_function(
                    FunctionStart {
                        name,
                        span: function.span,
                        parameters: function.params.items.len()
                            + usize::from(function.params.rest.is_some()),
                        boolean_parameters: boolean_parameter_count(&function.params),
                        async_function: function.r#async,
                        parameter_names: parameter_names(&function.params),
                        test_function,
                    },
                    function.get_function_body(),
                    function.get_expression(),
                );
            }
            other => self.record_fact(other),
        }
    }

    fn leave_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => {
                self.metrics.finish_function();
                self.parameter_scopes.pop();
            }
            other => self.leave_non_function(other),
        }
    }
}

impl TypeScriptVisitor<'_> {
    fn leave_non_function(&mut self, kind: AstKind<'_>) {
        match kind {
            AstKind::VariableDeclarator(_) => {
                self.declarator_hints.pop();
            }
            AstKind::MethodDefinition(_) => {
                self.method_hints.pop();
            }
            AstKind::ObjectProperty(_) => {
                self.property_hints.pop();
            }
            AstKind::IfStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::ForStatement(_)
            | AstKind::ForInStatement(_)
            | AstKind::ForOfStatement(_)
            | AstKind::SwitchStatement(_)
            | AstKind::CatchClause(_) => self.metrics.leave_control_flow(true, false),
            AstKind::ConditionalExpression(_) => self.metrics.leave_control_flow(true, true),
            AstKind::LogicalExpression(_) => self.metrics.leave_boolean_operator(),
            _ => {}
        }
    }
}

fn boolean_parameter_count(parameters: &FormalParameters<'_>) -> usize {
    parameters
        .items
        .iter()
        .filter(|parameter| {
            parameter
                .type_annotation
                .as_ref()
                .is_some_and(|annotation| {
                    matches!(annotation.type_annotation, TSType::TSBooleanKeyword(_))
                })
        })
        .count()
}

fn block_body_is_blank(source: &str, span: Span) -> bool {
    source
        .get(span.start as usize..span.end as usize)
        .and_then(|block| block.strip_prefix('{')?.strip_suffix('}'))
        .is_some_and(|body| body.trim().is_empty())
}

fn else_if_conditions(statement: &IfStatement<'_>) -> usize {
    let mut conditions = 1;
    let mut alternate = statement.alternate.as_ref();
    while let Some(Statement::IfStatement(next)) = alternate {
        conditions += 1;
        alternate = next.alternate.as_ref();
    }
    conditions
}

fn is_thin_wrapper(body: &FunctionBody<'_>, parameter_count: usize) -> bool {
    if parameter_count == 0 || body.statements.len() != 1 {
        return false;
    }

    let expression = match &body.statements[0] {
        Statement::ReturnStatement(statement) => statement.argument.as_ref(),
        Statement::ExpressionStatement(statement) => Some(&statement.expression),
        _ => None,
    };
    expression.is_some_and(|expression| is_thin_expression(expression, parameter_count))
}

fn is_thin_expression(expression: &Expression<'_>, parameter_count: usize) -> bool {
    direct_call(expression).is_some_and(|call| {
        call.arguments.len() == parameter_count
            && call
                .arguments
                .iter()
                .all(|argument| matches!(argument, Argument::Identifier(_)))
    })
}

fn direct_call<'borrow, 'ast>(
    expression: &'borrow Expression<'ast>,
) -> Option<&'borrow oxc_ast::ast::CallExpression<'ast>> {
    match expression {
        Expression::CallExpression(call) => Some(call),
        Expression::AwaitExpression(await_expression) => direct_call(&await_expression.argument),
        _ => None,
    }
}
