use ra_ap_syntax::{
    ast::{self, AstNode, HasArgList, HasModuleItem, HasName, HasVisibility},
    Edition, SyntaxKind, SyntaxNode, WalkEvent,
};

use crate::model::{DependencyKind, Language, ModuleDependency};

use super::clone_detection;
use super::core::{is_vague_name, FunctionInput, LineIndex, MetricCollector, ParsedFacts};
use super::rust_signals::{
    assignment_mutates_parameter, boolean_argument_count, boolean_parameter_count,
    is_assertion_macro, is_assignment_expression, is_error_laundering_match, is_panic_macro,
    is_panic_method, is_test_function, method_chain_metrics, method_mutates_parameter,
    mutable_input_names, result_return_is_assertion,
};
use super::rust_syntax::{
    else_if_conditions, has_try_modifier, is_logical_expression, is_thin_block, match_arm_count,
    parse_with_best_edition, requires_structural_parse, strip_delimiters,
};

const MACRO_RECURSION_LIMIT: usize = 4;
const STATEMENT_PREFIX: &str = "fn __slop_macro_fragment() {\n";
const EXPRESSION_PREFIX: &str = "fn __slop_macro_fragment() {\nlet _ = (";

pub(super) fn collect(source: &str) -> ParsedFacts {
    let (parse, edition) = parse_with_best_edition(source);
    let parse_errors = parse.errors().len();
    let lines = LineIndex::new(source);
    let mut visitor = RustVisitor::new(edition);
    visitor.metrics.facts.top_level_statements = parse.tree().items().count();
    visitor.walk(&parse.syntax_node(), LineMap::new(&lines, 0));

    ParsedFacts {
        facts: visitor.metrics.facts,
        parse_errors,
    }
}

struct RustVisitor {
    metrics: MetricCollector,
    macro_depth: usize,
    edition: Edition,
    parameter_scopes: Vec<Vec<String>>,
}

impl RustVisitor {
    fn new(edition: Edition) -> Self {
        Self {
            metrics: MetricCollector::default(),
            macro_depth: 0,
            edition,
            parameter_scopes: Vec::new(),
        }
    }

    fn walk(&mut self, root: &SyntaxNode, lines: LineMap<'_>) {
        for event in root.preorder() {
            match event {
                WalkEvent::Enter(node) => self.enter_node(&node, lines),
                WalkEvent::Leave(node) => self.leave_node(&node),
            }
        }
    }

    fn enter_node(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        self.enter_callable(node, lines);
        self.enter_flow(node, lines);
        self.record_declaration(node, lines);
        self.record_module_dependency(node, lines);
        self.record_macro(node, lines);
    }

    fn enter_callable(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        match node.kind() {
            SyntaxKind::FN => self.enter_function(node, lines),
            SyntaxKind::CLOSURE_EXPR => self.enter_closure(node, lines),
            _ => {}
        }
    }

    fn enter_flow(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        self.enter_control_node(node, lines);
        self.record_flow_event(node, lines);
    }

    fn enter_control_node(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        match node.kind() {
            SyntaxKind::IF_EXPR => {
                self.metrics.enter_control_flow(true, false);
                self.metrics.record_else_if_chain(else_if_conditions(node));
            }
            SyntaxKind::FOR_EXPR | SyntaxKind::WHILE_EXPR | SyntaxKind::LOOP_EXPR => {
                self.metrics.enter_control_flow(true, false)
            }
            SyntaxKind::MATCH_EXPR => {
                self.metrics.enter_control_flow(true, false);
                self.metrics.record_branch_fanout(match_arm_count(node));
                if is_error_laundering_match(node) {
                    self.metrics
                        .facts
                        .error_laundering
                        .push(lines.line(node.text_range().start().into()));
                }
            }
            SyntaxKind::BLOCK_EXPR if has_try_modifier(node) => {
                self.metrics.enter_control_flow(true, false);
            }
            _ => {}
        }
    }

    fn record_flow_event(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        match node.kind() {
            SyntaxKind::BIN_EXPR if is_logical_expression(node) => {
                self.metrics.enter_boolean_operator();
            }
            SyntaxKind::BIN_EXPR if is_assignment_expression(node) => {
                self.record_assignment(node);
            }
            SyntaxKind::AWAIT_EXPR => self.metrics.record_await_point(),
            SyntaxKind::METHOD_CALL_EXPR => self.record_method_call(node, lines),
            SyntaxKind::CALL_EXPR => self.record_boolean_call(node, lines),
            SyntaxKind::RETURN_EXPR | SyntaxKind::BREAK_EXPR | SyntaxKind::CONTINUE_EXPR => {
                self.metrics.record_exit_point();
            }
            _ => {}
        }
    }

    fn record_assignment(&mut self, node: &SyntaxNode) {
        self.metrics.record_mutation();
        if assignment_mutates_parameter(node, self.current_parameters()) {
            self.metrics.record_input_mutation();
        }
    }

    fn record_method_call(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        if is_panic_method(node) {
            self.metrics.record_panic_path();
        }
        let (steps, callbacks) = method_chain_metrics(node);
        self.metrics.record_chain(steps, callbacks);
        if method_mutates_parameter(node, self.current_parameters()) {
            self.metrics.record_input_mutation();
        }
        self.record_boolean_call(node, lines);
    }

    fn record_boolean_call(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        let arguments = match node.kind() {
            SyntaxKind::CALL_EXPR => {
                ast::CallExpr::cast(node.clone()).and_then(|call| call.arg_list())
            }
            SyntaxKind::METHOD_CALL_EXPR => {
                ast::MethodCallExpr::cast(node.clone()).and_then(|call| call.arg_list())
            }
            _ => None,
        };
        let count = boolean_argument_count(arguments);
        if count >= 3 {
            self.metrics
                .facts
                .boolean_literal_calls
                .push((lines.line(node.text_range().start().into()), count));
        }
    }

    fn current_parameters(&self) -> &[String] {
        self.parameter_scopes.last().map_or(&[], Vec::as_slice)
    }

    fn record_declaration(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        match node.kind() {
            SyntaxKind::IDENT_PAT => self.record_binding(node, lines),
            SyntaxKind::USE => self.metrics.facts.imports += 1,
            SyntaxKind::STRUCT => self.metrics.facts.structs += 1,
            SyntaxKind::ENUM => self.metrics.facts.enums += 1,
            SyntaxKind::TRAIT => self.metrics.facts.traits += 1,
            SyntaxKind::TYPE_ALIAS => self.metrics.facts.type_aliases += 1,
            _ => {}
        }
    }

    fn record_macro(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        match node.kind() {
            SyntaxKind::MACRO_CALL => {
                if is_assertion_macro(node) {
                    self.metrics.record_assertion();
                }
                if is_panic_macro(node) {
                    self.metrics.record_panic_path();
                }
                self.analyze_macro(node, lines);
            }
            SyntaxKind::MACRO_RULES => self.analyze_macro_rules(node, lines),
            SyntaxKind::MACRO_DEF => self.analyze_macro_definition(node, lines),
            _ => {}
        }
    }

    fn record_module_dependency(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        if node.kind() == SyntaxKind::USE {
            self.record_use_dependency(node, lines);
        }
    }

    fn record_use_dependency(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        let Some(import) = ast::Use::cast(node.clone()) else {
            return;
        };
        let Some(tree) = import.use_tree() else {
            return;
        };
        self.metrics.facts.dependencies.push(ModuleDependency {
            specifier: tree.syntax().text().to_string(),
            line: lines.line(node.text_range().start().into()),
            kind: if import.visibility().is_some() {
                DependencyKind::ReExport
            } else {
                DependencyKind::Import
            },
        });
    }

    fn leave_node(&mut self, node: &SyntaxNode) {
        match node.kind() {
            SyntaxKind::FN | SyntaxKind::CLOSURE_EXPR => {
                self.metrics.finish_function();
                self.parameter_scopes.pop();
            }
            SyntaxKind::IF_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::MATCH_EXPR => self.metrics.leave_control_flow(true, false),
            SyntaxKind::BLOCK_EXPR if has_try_modifier(node) => {
                self.metrics.leave_control_flow(true, false);
            }
            SyntaxKind::BIN_EXPR if is_logical_expression(node) => {
                self.metrics.leave_boolean_operator();
            }
            _ => {}
        }
    }

    fn enter_function(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        let Some(function) = ast::Fn::cast(node.clone()) else {
            return;
        };
        let body = function.body();
        let parameter_list = function.param_list();
        let parameters = parameter_list
            .as_ref()
            .map_or(0, |parameters| parameters.params().count());
        let boolean_parameters = parameter_list
            .as_ref()
            .map_or(0, |parameters| boolean_parameter_count(parameters.params()));
        let mutable_inputs = mutable_input_names(parameter_list);
        let range = node.text_range();
        let start_line = lines.line(range.start().into());
        let end_line = lines.line(range.end().into());
        let thin_wrapper = body.as_ref().is_some_and(|body| {
            end_line.saturating_sub(start_line) < 6 && is_thin_block(body, parameters)
        });
        if self.macro_depth == 0 && body.is_some() {
            let source = node.text().to_string();
            if let Some(candidate) =
                clone_detection::candidate(&source, Language::Rust, start_line, end_line)
            {
                self.metrics.facts.clone_candidates.push(candidate);
            }
        }
        let test_function = is_test_function(&function);
        let result_assertion = test_function && result_return_is_assertion(&function);
        self.parameter_scopes.push(mutable_inputs);
        self.metrics.start_function(FunctionInput {
            name: function.name().map(|name| name.to_string()),
            start_line,
            end_line,
            parameters,
            boolean_parameters,
            async_function: function.async_token().is_some(),
            test_function,
            input_mutation_threshold: 3,
            thin_wrapper,
            has_body: body.is_some(),
        });
        if result_assertion {
            self.metrics.record_assertion();
        }
    }

    fn enter_closure(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        let Some(closure) = ast::ClosureExpr::cast(node.clone()) else {
            return;
        };
        let range = node.text_range();
        let parameters = closure.param_list();
        let mutable_inputs = mutable_input_names(parameters.clone());
        self.parameter_scopes.push(mutable_inputs);
        self.metrics.start_function(FunctionInput {
            name: None,
            start_line: lines.line(range.start().into()),
            end_line: lines.line(range.end().into()),
            parameters: parameters
                .as_ref()
                .map_or(0, |parameters| parameters.params().count()),
            boolean_parameters: parameters
                .map_or(0, |parameters| boolean_parameter_count(parameters.params())),
            async_function: closure.async_token().is_some(),
            test_function: false,
            input_mutation_threshold: 3,
            thin_wrapper: false,
            has_body: closure.body().is_some(),
        });
    }

    fn record_binding(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        let Some(binding) = ast::IdentPat::cast(node.clone()) else {
            return;
        };
        let Some(name) = binding.name().map(|name| name.to_string()) else {
            return;
        };
        if is_vague_name(&name) {
            self.metrics
                .facts
                .vague_bindings
                .push((name, lines.line(node.text_range().start().into())));
        }
    }

    fn analyze_macro(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        self.metrics.facts.macro_invocations += 1;
        let analyzed = ast::MacroCall::cast(node.clone())
            .and_then(|call| call.token_tree())
            .is_some_and(|tree| self.analyze_token_tree(&tree, lines));
        if analyzed {
            self.metrics.facts.macro_inputs_analyzed += 1;
        } else {
            self.metrics.facts.macro_inputs_unresolved += 1;
        }
    }

    fn analyze_macro_rules(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        self.metrics.facts.macro_definitions += 1;
        let analyzed = ast::MacroRules::cast(node.clone())
            .and_then(|definition| definition.token_tree())
            .is_some_and(|tree| self.analyze_token_tree(&tree, lines));
        self.record_macro_coverage(analyzed);
    }

    fn analyze_macro_definition(&mut self, node: &SyntaxNode, lines: LineMap<'_>) {
        self.metrics.facts.macro_definitions += 1;
        let analyzed = ast::MacroDef::cast(node.clone())
            .and_then(|definition| definition.body())
            .is_some_and(|tree| self.analyze_token_tree(&tree, lines));
        self.record_macro_coverage(analyzed);
    }

    fn record_macro_coverage(&mut self, analyzed: bool) {
        if analyzed {
            self.metrics.facts.macro_inputs_analyzed += 1;
        } else {
            self.metrics.facts.macro_inputs_unresolved += 1;
        }
    }

    fn analyze_token_tree(&mut self, tree: &ast::TokenTree, lines: LineMap<'_>) -> bool {
        if self.macro_depth >= MACRO_RECURSION_LIMIT {
            return false;
        }
        let syntax = tree.syntax();
        let text = syntax.text().to_string();
        let Some(body) = strip_delimiters(&text) else {
            return false;
        };
        let body_line = lines.line(u32::from(syntax.text_range().start()).saturating_add(1));

        self.macro_depth += 1;
        let analyzed = if self.try_macro_fragment(body, body_line) {
            true
        } else {
            let mut child_analyzed = false;
            for child in syntax.children().filter_map(ast::TokenTree::cast) {
                child_analyzed |= self.analyze_token_tree(&child, lines);
            }
            child_analyzed
        };
        self.macro_depth -= 1;
        analyzed
    }

    fn try_macro_fragment(&mut self, body: &str, body_line: usize) -> bool {
        if !requires_structural_parse(body) {
            return true;
        }
        self.try_item_fragment(body, body_line)
            || self.try_statement_fragment(body, body_line)
            || self.try_expression_fragment(body, body_line)
    }

    fn try_item_fragment(&mut self, body: &str, body_line: usize) -> bool {
        let parse = ast::SourceFile::parse(body, self.edition);
        if !parse.errors().is_empty() || parse.tree().items().next().is_none() {
            return false;
        }
        let local_lines = LineIndex::new(body);
        self.walk(
            &parse.syntax_node(),
            LineMap::new(&local_lines, body_line as isize - 1),
        );
        true
    }

    fn try_statement_fragment(&mut self, body: &str, body_line: usize) -> bool {
        let source = format!("{STATEMENT_PREFIX}{body}\n}}");
        self.walk_wrapped_fragment(&source, STATEMENT_PREFIX.len(), body_line)
    }

    fn try_expression_fragment(&mut self, body: &str, body_line: usize) -> bool {
        let source = format!("{EXPRESSION_PREFIX}{body});\n}}");
        self.walk_wrapped_fragment(&source, EXPRESSION_PREFIX.len(), body_line)
    }

    fn walk_wrapped_fragment(
        &mut self,
        source: &str,
        body_offset: usize,
        body_line: usize,
    ) -> bool {
        let parse = ast::SourceFile::parse(source, self.edition);
        if !parse.errors().is_empty() {
            return false;
        }
        let root = parse.syntax_node();
        let Some(statements) = root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::STMT_LIST)
        else {
            return false;
        };
        let local_lines = LineIndex::new(source);
        let local_body_line = local_lines.line(body_offset as u32);
        self.walk(
            &statements,
            LineMap::new(&local_lines, body_line as isize - local_body_line as isize),
        );
        true
    }
}

#[derive(Clone, Copy)]
struct LineMap<'a> {
    lines: &'a LineIndex,
    adjustment: isize,
}

impl<'a> LineMap<'a> {
    const fn new(lines: &'a LineIndex, adjustment: isize) -> Self {
        Self { lines, adjustment }
    }

    fn line(self, offset: u32) -> usize {
        (self.lines.line(offset) as isize + self.adjustment).max(1) as usize
    }
}
