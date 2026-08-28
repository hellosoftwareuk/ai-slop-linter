use std::collections::{HashSet, VecDeque};

use oxc_ast::{
    ast::{Expression, PropertyKind, VariableDeclarationKind},
    AstKind,
};
use oxc_semantic::{AstNodes, NodeId, Semantic, SymbolId};
use oxc_span::{GetSpan, Span};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Owner {
    symbol: SymbolId,
    path: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub span: Span,
    pub replacement: String,
}

struct ProofContext<'a, 's> {
    semantic: &'a Semantic<'s>,
    new_key: &'a str,
    source: &'a str,
}

struct ReferenceUse {
    node_id: NodeId,
    is_write: bool,
}

pub(super) fn owner_for_object(
    semantic: &Semantic<'_>,
    mut object_id: NodeId,
    mut path: Vec<String>,
) -> Result<Owner, &'static str> {
    let nodes = semantic.nodes();
    loop {
        let parent_id = nodes.parent_id(object_id);
        match nodes.kind(parent_id) {
            AstKind::ObjectProperty(property)
                if property.kind == PropertyKind::Init
                    && !property.computed
                    && !property.method
                    && property.value.span() == nodes.kind(object_id).span() =>
            {
                let Some(name) = super::typescript_key_remap::static_property_name(property) else {
                    return Err("a containing property is dynamic");
                };
                path.insert(0, name.to_owned());
                let parent_object_id = nodes.parent_id(parent_id);
                if !matches!(nodes.kind(parent_object_id), AstKind::ObjectExpression(_)) {
                    return Err("the containing value has no plain object owner");
                }
                object_id = parent_object_id;
            }
            AstKind::VariableDeclarator(declarator)
                if declarator
                    .init
                    .as_ref()
                    .is_some_and(|init| init.span() == nodes.kind(object_id).span()) =>
            {
                return owner_from_declarator(nodes, parent_id, declarator, path);
            }
            AstKind::CallExpression(_) | AstKind::NewExpression(_) => {
                return Err("the object literal is passed directly to a call");
            }
            AstKind::ReturnStatement(_) | AstKind::YieldExpression(_) => {
                return Err("the object literal is returned from its scope");
            }
            _ => return Err("the object literal has no closed local const owner"),
        }
    }
}

pub(super) fn prove(
    semantic: &Semantic<'_>,
    initial: Owner,
    new_key: &str,
    source: &str,
) -> Result<Vec<Edit>, &'static str> {
    let mut pending = VecDeque::from([initial]);
    let mut visited = HashSet::new();
    let mut edits = Vec::new();
    let context = ProofContext {
        semantic,
        new_key,
        source,
    };

    while let Some(owner) = pending.pop_front() {
        if !visited.insert(owner.clone()) {
            continue;
        }
        if visited.len() > 64 {
            return Err("the alias and containment graph is too wide to prove locally");
        }
        for reference in semantic.symbol_references(owner.symbol) {
            if let Some(next) = follow_reference(
                &context,
                &owner,
                ReferenceUse {
                    node_id: reference.node_id(),
                    is_write: reference.is_write(),
                },
                &mut edits,
            )? {
                pending.push_back(next);
            }
        }
    }

    Ok(edits)
}

fn follow_reference(
    context: &ProofContext<'_, '_>,
    owner: &Owner,
    reference: ReferenceUse,
    edits: &mut Vec<Edit>,
) -> Result<Option<Owner>, &'static str> {
    if reference.is_write {
        return Err("the owning binding is reassigned");
    }
    let semantic = context.semantic;
    let reference_id = reference.node_id;
    if let Some(symbol) = direct_const_alias(semantic.nodes(), reference_id)? {
        return Ok(Some(Owner {
            symbol,
            path: owner.path.clone(),
        }));
    }
    if let Some(contained) = contained_owner(semantic, reference_id, &owner.path)? {
        return Ok(Some(contained));
    }
    if let Some(symbol) = local_call_parameter(semantic, reference_id)? {
        return Ok(Some(Owner {
            symbol,
            path: owner.path.clone(),
        }));
    }
    match static_access(semantic.nodes(), reference_id, context.source)? {
        Access::Path { steps, dynamic } => {
            assess_path(&owner.path, &steps, dynamic, context.new_key, edits)?;
            Ok(None)
        }
        Access::Bare => Err(escape_reason(semantic.nodes(), reference_id)),
    }
}

fn local_call_parameter(
    semantic: &Semantic<'_>,
    reference_id: NodeId,
) -> Result<Option<SymbolId>, &'static str> {
    let Some((callee_symbol, argument_index)) = local_call_target(semantic, reference_id)? else {
        return Ok(None);
    };
    local_parameter_symbol(semantic, callee_symbol, argument_index).map(Some)
}

fn local_call_target(
    semantic: &Semantic<'_>,
    reference_id: NodeId,
) -> Result<Option<(SymbolId, usize)>, &'static str> {
    let nodes = semantic.nodes();
    let call_id = nodes.parent_id(reference_id);
    let AstKind::CallExpression(call) = nodes.kind(call_id) else {
        return Ok(None);
    };
    let reference_span = nodes.kind(reference_id).span();
    let Some(argument_index) = call
        .arguments
        .iter()
        .position(|argument| !argument.is_spread() && argument.span() == reference_span)
    else {
        return Ok(None);
    };
    let Expression::Identifier(callee) = &call.callee else {
        return Err("the object reaches a method or dynamic function call");
    };
    let Some(callee_reference) = callee.reference_id.get() else {
        return Err("the called function has no resolved symbol");
    };
    let Some(callee_symbol) = semantic
        .scoping()
        .get_reference(callee_reference)
        .symbol_id()
    else {
        return Err("the object reaches an external function call");
    };
    let mut call_sites = semantic.symbol_references(callee_symbol);
    let Some(only_call) = call_sites.next() else {
        return Err("the called function has no traceable call site");
    };
    if call_sites.next().is_some() || only_call.node_id() != callee.node_id.get() {
        return Err("the called function has multiple callers or escapes its scope");
    }

    Ok(Some((callee_symbol, argument_index)))
}

fn local_parameter_symbol(
    semantic: &Semantic<'_>,
    callee_symbol: SymbolId,
    argument_index: usize,
) -> Result<SymbolId, &'static str> {
    let nodes = semantic.nodes();
    let declaration = semantic.symbol_declaration(callee_symbol);
    let declaration_id = declaration.id();
    let (parameters, function_id) = match declaration.kind() {
        AstKind::Function(function) => (&*function.params, function.node_id.get()),
        AstKind::VariableDeclarator(declarator) => function_from_declarator(nodes, declarator)?,
        AstKind::BindingIdentifier(_) => match nodes.parent_kind(declaration_id) {
            AstKind::Function(function) => (&*function.params, function.node_id.get()),
            AstKind::VariableDeclarator(declarator) => function_from_declarator(nodes, declarator)?,
            _ => return Err("the called symbol is not a locally analyzable function"),
        },
        _ => return Err("the called symbol is not a locally analyzable function"),
    };
    if !nodes
        .ancestor_kinds(function_id)
        .any(|kind| matches!(kind, AstKind::FunctionBody(_)))
    {
        return Err("the called function is outside the closed local boundary");
    }
    let Some(parameter) = parameters.items.get(argument_index) else {
        return Err("the call is handled by a rest or missing parameter");
    };
    if parameter.type_annotation.is_some()
        || parameter.initializer.is_some()
        || parameter.optional
        || !parameter.decorators.is_empty()
    {
        return Err("the receiving parameter carries a type or runtime contract");
    }
    let Some(binding) = parameter.pattern.get_binding_identifier() else {
        return Err("the receiving parameter uses destructuring");
    };
    binding
        .symbol_id
        .get()
        .ok_or("the receiving parameter has no resolved symbol")
}

fn function_from_declarator<'a>(
    nodes: &AstNodes<'a>,
    declarator: &'a oxc_ast::ast::VariableDeclarator<'a>,
) -> Result<(&'a oxc_ast::ast::FormalParameters<'a>, NodeId), &'static str> {
    let declaration_node = nodes.parent_kind(declarator.node_id.get());
    if !matches!(
        declaration_node,
        AstKind::VariableDeclaration(declaration)
            if declaration.kind == VariableDeclarationKind::Const
    ) {
        return Err("the called function binding is mutable");
    }
    match declarator.init.as_ref() {
        Some(Expression::ArrowFunctionExpression(function)) => {
            Ok((&function.params, function.node_id.get()))
        }
        Some(Expression::FunctionExpression(function)) => {
            Ok((&function.params, function.node_id.get()))
        }
        _ => Err("the called symbol is not a locally analyzable function"),
    }
}

fn owner_from_declarator(
    nodes: &AstNodes<'_>,
    declarator_id: NodeId,
    declarator: &oxc_ast::ast::VariableDeclarator<'_>,
    path: Vec<String>,
) -> Result<Owner, &'static str> {
    let declaration_id = nodes.parent_id(declarator_id);
    let AstKind::VariableDeclaration(declaration) = nodes.kind(declaration_id) else {
        return Err("the object owner is not a variable declaration");
    };
    if declaration.kind != VariableDeclarationKind::Const {
        return Err("the object owner is not const");
    }
    if declarator.type_annotation.is_some() {
        return Err("the object owner has an explicit type contract");
    }
    let Some(binding) = declarator.id.get_binding_identifier() else {
        return Err("the object owner uses a destructuring binding");
    };
    let Some(symbol) = binding.symbol_id.get() else {
        return Err("the object owner has no resolved symbol");
    };
    if !nodes
        .ancestor_kinds(declarator_id)
        .any(|kind| matches!(kind, AstKind::FunctionBody(_)))
    {
        return Err("the object owner is outside a function-local boundary");
    }
    Ok(Owner { symbol, path })
}

fn direct_const_alias(
    nodes: &AstNodes<'_>,
    reference_id: NodeId,
) -> Result<Option<SymbolId>, &'static str> {
    let parent_id = nodes.parent_id(reference_id);
    let AstKind::VariableDeclarator(declarator) = nodes.kind(parent_id) else {
        return Ok(None);
    };
    if declarator
        .init
        .as_ref()
        .is_none_or(|init| init.span() != nodes.kind(reference_id).span())
    {
        return Ok(None);
    }
    let declaration_id = nodes.parent_id(parent_id);
    let AstKind::VariableDeclaration(declaration) = nodes.kind(declaration_id) else {
        return Err("an alias has no ordinary variable declaration");
    };
    if declaration.kind != VariableDeclarationKind::Const || declarator.type_annotation.is_some() {
        return Err("an alias is mutable or carries a type contract");
    }
    let Some(binding) = declarator.id.get_binding_identifier() else {
        return Err("an alias uses destructuring");
    };
    binding
        .symbol_id
        .get()
        .map(Some)
        .ok_or("an alias has no resolved symbol")
}

fn contained_owner(
    semantic: &Semantic<'_>,
    reference_id: NodeId,
    path: &[String],
) -> Result<Option<Owner>, &'static str> {
    let nodes = semantic.nodes();
    let property_id = nodes.parent_id(reference_id);
    let AstKind::ObjectProperty(property) = nodes.kind(property_id) else {
        return Ok(None);
    };
    if property.kind != PropertyKind::Init
        || property.computed
        || property.method
        || property.value.span() != nodes.kind(reference_id).span()
    {
        return Ok(None);
    }
    let Some(name) = super::typescript_key_remap::static_property_name(property) else {
        return Err("an alias is stored behind a dynamic property");
    };
    let object_id = nodes.parent_id(property_id);
    if !matches!(nodes.kind(object_id), AstKind::ObjectExpression(_)) {
        return Err("an alias is stored in an untracked container");
    }
    let mut contained_path = Vec::with_capacity(path.len() + 1);
    contained_path.push(name.to_owned());
    contained_path.extend(path.iter().cloned());
    owner_for_object(semantic, object_id, contained_path).map(Some)
}

enum Access {
    Bare,
    Path {
        steps: Vec<AccessStep>,
        dynamic: bool,
    },
}

struct AccessStep {
    key: String,
    edit_span: Span,
    style: AccessStyle,
}

enum AccessStyle {
    Static,
    Computed { optional: bool },
}

fn static_access(
    nodes: &AstNodes<'_>,
    reference_id: NodeId,
    source: &str,
) -> Result<Access, &'static str> {
    let mut current_id = reference_id;
    let mut steps = Vec::new();
    let mut dynamic = false;
    loop {
        let parent_id = nodes.parent_id(current_id);
        let current_span = nodes.kind(current_id).span();
        match nodes.kind(parent_id) {
            AstKind::StaticMemberExpression(member) if member.object.span() == current_span => {
                steps.push(AccessStep {
                    key: member.property.name.to_string(),
                    edit_span: member.property.span,
                    style: AccessStyle::Static,
                });
                current_id = parent_id;
            }
            AstKind::ComputedMemberExpression(member) if member.object.span() == current_span => {
                let Expression::StringLiteral(literal) = &member.expression else {
                    dynamic = true;
                    break;
                };
                let text = source
                    .get(literal.span.start as usize..literal.span.end as usize)
                    .ok_or("a computed key has an invalid source range")?;
                let Some(quote) = text
                    .chars()
                    .next()
                    .filter(|quote| matches!(quote, '\'' | '"'))
                else {
                    return Err("a computed key is not a plain string literal");
                };
                if !text.ends_with(quote)
                    || text.get(1..text.len().saturating_sub(1)) != Some(literal.value.as_str())
                {
                    return Err("a computed key contains escapes");
                }
                steps.push(AccessStep {
                    key: literal.value.to_string(),
                    edit_span: Span::new(member.object.span().end, member.span.end),
                    style: AccessStyle::Computed {
                        optional: member.optional,
                    },
                });
                current_id = parent_id;
            }
            AstKind::ChainExpression(chain) if chain.expression.span() == current_span => {
                current_id = parent_id;
            }
            _ => break,
        }
    }
    if steps.is_empty() && !dynamic {
        Ok(Access::Bare)
    } else {
        Ok(Access::Path { steps, dynamic })
    }
}

fn assess_path(
    expected: &[String],
    actual: &[AccessStep],
    dynamic: bool,
    new_key: &str,
    edits: &mut Vec<Edit>,
) -> Result<(), &'static str> {
    let common = expected
        .iter()
        .zip(actual)
        .take_while(|(left, right)| left.as_str() == right.key)
        .count();
    if common == expected.len() {
        let target = &actual[expected.len() - 1];
        let replacement = match target.style {
            AccessStyle::Static => new_key.to_owned(),
            AccessStyle::Computed { optional: false } => format!(".{new_key}"),
            AccessStyle::Computed { optional: true } => format!("?.{new_key}"),
        };
        edits.push(Edit {
            span: target.edit_span,
            replacement,
        });
        return Ok(());
    }
    if common < actual.len() {
        return Ok(());
    }
    if dynamic {
        Err("a containing object is accessed with a computed key")
    } else {
        Err("a containing object is observed before the remapped property")
    }
}

fn escape_reason(nodes: &AstNodes<'_>, reference_id: NodeId) -> &'static str {
    for kind in nodes.ancestor_kinds(reference_id).take(6) {
        match kind {
            AstKind::CallExpression(_) | AstKind::NewExpression(_) => {
                return "the object reaches a function or constructor call";
            }
            AstKind::ReturnStatement(_) | AstKind::YieldExpression(_) => {
                return "the object is returned from its scope";
            }
            AstKind::SpreadElement(_) | AstKind::JSXSpreadAttribute(_) => {
                return "the object is spread into another value";
            }
            AstKind::TSTypeQuery(_) => return "the object participates in a type contract",
            _ => {}
        }
    }
    "the object is used without a fully static property path"
}
