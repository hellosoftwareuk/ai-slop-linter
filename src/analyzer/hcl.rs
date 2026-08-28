use anyhow::{anyhow, Result};
use tree_sitter::{Node, Parser};

use crate::model::{DependencyKind, Language, ModuleDependency};

use super::{
    clone_detection,
    core::{Facts, HclBlockMetrics, ParsedFacts},
};

pub(super) fn collect(source: &str, language: Language) -> Result<ParsedFacts> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_hcl::LANGUAGE.into())
        .map_err(|error| anyhow!("cannot load HCL grammar: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("HCL parser did not produce a syntax tree"))?;
    let root = tree.root_node();
    let mut collector = HclCollector {
        source,
        language,
        facts: Facts::default(),
    };
    collector.facts.top_level_statements = top_level_statement_count(root);
    collector.visit(root, 0);
    Ok(ParsedFacts {
        facts: collector.facts,
        parse_errors: syntax_error_count(root),
    })
}

struct HclCollector<'a> {
    source: &'a str,
    language: Language,
    facts: Facts,
}

impl HclCollector<'_> {
    fn visit(&mut self, node: Node<'_>, parent_block_depth: usize) {
        let block_depth = if node.kind() == "block" {
            let depth = parent_block_depth + 1;
            self.record_block(node, depth);
            depth
        } else {
            parent_block_depth
        };
        if self.language == Language::Terragrunt && node.kind() == "function_call" {
            self.record_terragrunt_call(node);
        }
        for child in named_children(node) {
            self.visit(child, block_depth);
        }
    }

    fn record_block(&mut self, node: Node<'_>, depth: usize) {
        let (block_type, label) = block_header(node, self.source);
        let body = direct_child(node, "body");
        let attributes = body.map_or_else(Vec::new, |body| direct_children(body, "attribute"));
        let nested_blocks = body.map_or_else(Vec::new, |body| direct_children(body, "block"));
        let expression = strongest_expression(&attributes, self.source);
        let line = node.start_position().row + 1;
        let end_line = node.end_position().row + 1;
        let metrics = HclBlockMetrics {
            block_type: block_type.clone(),
            label: label.clone(),
            line,
            lines: end_line.saturating_sub(line) + 1,
            depth,
            attributes: attributes.len(),
            nested_blocks: nested_blocks.len(),
            dynamic_blocks: body.map_or(0, |body| {
                descendant_block_count(body, self.source, "dynamic")
            }),
            max_expression_complexity: expression.complexity,
            expression_line: expression.line,
            max_collection_items: expression.collection_items,
            collection_line: expression.collection_line,
        };

        self.record_interface(&block_type, &label, line, &attributes);
        self.record_source(&block_type, line, &attributes);
        self.record_lifecycle(&block_type, &attributes);
        self.record_explicit_dependencies(&attributes);
        self.record_terragrunt_block(&block_type, line, &attributes);
        if depth == 1 {
            let block_source = node_text(node, self.source);
            if let Some(candidate) =
                clone_detection::candidate(block_source, self.language, line, end_line)
            {
                self.facts.clone_candidates.push(candidate);
            }
        }
        self.facts.hcl_blocks.push(metrics);
    }

    fn record_interface(
        &mut self,
        block_type: &str,
        label: &str,
        line: usize,
        attributes: &[Node<'_>],
    ) {
        if self.language != Language::Terraform || !matches!(block_type, "variable" | "output") {
            return;
        }
        if block_type == "variable" && find_attribute(attributes, self.source, "type").is_none() {
            self.facts.untyped_variables.push((label.to_owned(), line));
        }
        if find_attribute(attributes, self.source, "description").is_none() {
            self.facts
                .undocumented_interfaces
                .push((format!("{block_type}.{label}"), line));
        }
    }

    fn record_source(&mut self, block_type: &str, line: usize, attributes: &[Node<'_>]) {
        let source_owner = (self.language == Language::Terraform && block_type == "module")
            || (self.language == Language::Terragrunt && block_type == "terraform");
        if !source_owner {
            return;
        }
        let Some(source_attribute) = find_attribute(attributes, self.source, "source") else {
            return;
        };
        let Some(source) = static_attribute_string(source_attribute, self.source) else {
            return;
        };
        if is_local_source(&source) {
            self.record_dependency(source, line);
            return;
        }
        let has_version = find_attribute(attributes, self.source, "version").is_some();
        if is_floating_source(&source, has_version) {
            self.facts.floating_sources.push((source, line));
        }
    }

    fn record_lifecycle(&mut self, block_type: &str, attributes: &[Node<'_>]) {
        if block_type != "lifecycle" {
            return;
        }
        let Some(attribute) = find_attribute(attributes, self.source, "ignore_changes") else {
            return;
        };
        let value = attribute_value_text(attribute, self.source);
        if value.trim() == "all"
            || value
                .split(|character: char| !is_identifier_char(character))
                .any(|part| part == "all")
        {
            self.facts
                .broad_ignore_changes
                .push(attribute.start_position().row + 1);
        }
    }

    fn record_explicit_dependencies(&mut self, attributes: &[Node<'_>]) {
        let Some(attribute) = find_attribute(attributes, self.source, "depends_on") else {
            return;
        };
        let count = collection_item_count(attribute_value(attribute));
        if count >= 5 {
            self.facts
                .wide_explicit_dependencies
                .push((attribute.start_position().row + 1, count));
        }
    }

    fn record_terragrunt_block(&mut self, block_type: &str, line: usize, attributes: &[Node<'_>]) {
        if self.language != Language::Terragrunt {
            return;
        }
        match block_type {
            "dependency" => self.record_dependency_block(line, attributes),
            "dependencies" => self.record_dependencies_block(line, attributes),
            "include" => self.record_include_block(line, attributes),
            "before_hook" | "after_hook" | "error_hook" => {
                self.facts.terragrunt_hooks.push(line);
            }
            _ => {}
        }
    }

    fn record_dependency_block(&mut self, line: usize, attributes: &[Node<'_>]) {
        self.facts.terragrunt_dependencies.push(line);
        self.record_static_attribute_dependency(attributes, "config_path");
    }

    fn record_dependencies_block(&mut self, line: usize, attributes: &[Node<'_>]) {
        self.facts.terragrunt_dependencies.push(line);
        let Some(paths) = find_attribute(attributes, self.source, "paths") else {
            return;
        };
        let Some(value) = attribute_value(paths) else {
            return;
        };
        for path in static_strings(value, self.source) {
            self.record_dependency(path, paths.start_position().row + 1);
        }
    }

    fn record_include_block(&mut self, line: usize, attributes: &[Node<'_>]) {
        self.facts.terragrunt_includes.push(line);
        self.record_static_attribute_dependency(attributes, "path");
    }

    fn record_static_attribute_dependency(&mut self, attributes: &[Node<'_>], name: &str) {
        let Some(attribute) = find_attribute(attributes, self.source, name) else {
            return;
        };
        if let Some(path) = static_attribute_string(attribute, self.source) {
            self.record_dependency(path, attribute.start_position().row + 1);
        }
    }

    fn record_terragrunt_call(&mut self, node: Node<'_>) {
        if function_name(node, self.source) != "read_terragrunt_config" {
            return;
        }
        let line = node.start_position().row + 1;
        self.facts.terragrunt_config_reads.push(line);
        if let Some(path) = static_strings(node, self.source).into_iter().next() {
            self.record_dependency(path, line);
        }
    }

    fn record_dependency(&mut self, path: String, line: usize) {
        if !is_local_source(&path) {
            return;
        }
        self.facts.imports += 1;
        self.facts.dependencies.push(ModuleDependency {
            specifier: path,
            line,
            kind: DependencyKind::Import,
        });
    }
}

#[derive(Default)]
struct ExpressionStrength {
    complexity: usize,
    line: usize,
    collection_items: usize,
    collection_line: usize,
}

fn strongest_expression(attributes: &[Node<'_>], source: &str) -> ExpressionStrength {
    let mut strongest = ExpressionStrength::default();
    for attribute in attributes {
        let Some(value) = attribute_value(*attribute) else {
            continue;
        };
        let mut current = ExpressionStrength {
            line: value.start_position().row + 1,
            ..ExpressionStrength::default()
        };
        expression_strength(value, 0, &mut current);
        if current.complexity > strongest.complexity {
            strongest.complexity = current.complexity;
            strongest.line = current.line;
        }
        if current.collection_items > strongest.collection_items {
            strongest.collection_items = current.collection_items;
            strongest.collection_line = current.collection_line;
        }
    }
    let _ = source;
    strongest
}

fn expression_strength(node: Node<'_>, nesting: usize, strength: &mut ExpressionStrength) {
    let control = matches!(
        node.kind(),
        "conditional" | "for_expr" | "template_if" | "template_for"
    );
    let weight = match node.kind() {
        "conditional" | "for_expr" => 3 + nesting,
        "template_if" | "template_for" => 2 + nesting,
        "binary_operation" | "function_call" => 1,
        _ => 0,
    };
    strength.complexity += weight;
    if matches!(node.kind(), "tuple" | "object") {
        let items = collection_item_count(Some(node));
        if items > strength.collection_items {
            strength.collection_items = items;
            strength.collection_line = node.start_position().row + 1;
        }
    }
    let child_nesting = nesting + usize::from(control);
    for child in named_children(node) {
        expression_strength(child, child_nesting, strength);
    }
}

fn block_header(node: Node<'_>, source: &str) -> (String, String) {
    let children = named_children(node);
    let block_type = children
        .iter()
        .find(|child| child.kind() == "identifier")
        .map_or_else(String::new, |child| node_text(*child, source).to_owned());
    let label = children
        .iter()
        .find(|child| child.kind() == "string_lit")
        .map_or_else(String::new, |child| {
            unquote(node_text(*child, source)).to_owned()
        });
    (block_type, label)
}

fn find_attribute<'tree>(
    attributes: &[Node<'tree>],
    source: &str,
    name: &str,
) -> Option<Node<'tree>> {
    attributes.iter().copied().find(|attribute| {
        direct_child(*attribute, "identifier")
            .is_some_and(|identifier| node_text(identifier, source) == name)
    })
}

fn attribute_value(attribute: Node<'_>) -> Option<Node<'_>> {
    named_children(attribute)
        .into_iter()
        .find(|child| child.kind() != "identifier")
}

fn attribute_value_text<'a>(attribute: Node<'_>, source: &'a str) -> &'a str {
    attribute_value(attribute).map_or("", |value| node_text(value, source))
}

fn static_attribute_string(attribute: Node<'_>, source: &str) -> Option<String> {
    static_strings(attribute_value(attribute)?, source)
        .into_iter()
        .next()
}

fn static_strings(node: Node<'_>, source: &str) -> Vec<String> {
    let mut values = Vec::new();
    collect_static_strings(node, source, &mut values);
    values
}

fn collect_static_strings(node: Node<'_>, source: &str, values: &mut Vec<String>) {
    if matches!(node.kind(), "quoted_template" | "string_lit") {
        let text = node_text(node, source);
        if !text.contains("${") && !text.contains("%{") {
            values.push(unquote(text).to_owned());
        }
        return;
    }
    for child in named_children(node) {
        collect_static_strings(child, source, values);
    }
}

fn function_name<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    direct_child(node, "identifier").map_or("", |identifier| node_text(identifier, source))
}

fn collection_item_count(node: Option<Node<'_>>) -> usize {
    let Some(node) = node else {
        return 0;
    };
    match node.kind() {
        "object" => direct_children(node, "object_elem").len(),
        "tuple" => direct_children(node, "expression").len(),
        _ => named_children(node)
            .into_iter()
            .map(|child| collection_item_count(Some(child)))
            .max()
            .unwrap_or_default(),
    }
}

fn descendant_block_count(node: Node<'_>, source: &str, block_type: &str) -> usize {
    named_children(node)
        .into_iter()
        .map(|child| {
            let own =
                usize::from(child.kind() == "block" && block_header(child, source).0 == block_type);
            own + descendant_block_count(child, source, block_type)
        })
        .sum()
}

fn is_local_source(source: &str) -> bool {
    source.starts_with('.') || source.starts_with('/') || source.starts_with("file://")
}

fn is_floating_source(source: &str, has_version: bool) -> bool {
    let lowercase = source.to_ascii_lowercase();
    is_floating_git_source(&lowercase)
        || is_floating_tfr_source(&lowercase, has_version)
        || is_floating_registry_source(&lowercase, has_version)
}

fn is_floating_git_source(source: &str) -> bool {
    const GIT_PREFIXES: &[&str] = &["git::", "git@"];
    const GIT_HOSTS: &[&str] = &["github.com/", "gitlab.com/"];
    let is_git = GIT_PREFIXES.iter().any(|prefix| source.starts_with(prefix))
        || GIT_HOSTS.iter().any(|host| source.contains(host));
    is_git && !["?ref=", "&ref="].iter().any(|pin| source.contains(pin))
}

fn is_floating_tfr_source(source: &str, has_version: bool) -> bool {
    source.starts_with("tfr://") && !has_version && !source.contains("?version=")
}

fn is_floating_registry_source(source: &str, has_version: bool) -> bool {
    !has_version && !source.contains("://") && source.split('/').count() >= 3
}

fn is_identifier_char(character: char) -> bool {
    character == '_' || character == '-' || character.is_alphanumeric()
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

fn direct_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == kind)
}

fn direct_children<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    named_children(node)
        .into_iter()
        .filter(|child| child.kind() == kind)
        .collect()
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

fn top_level_statement_count(root: Node<'_>) -> usize {
    direct_child(root, "body").map_or(0, |body| named_children(body).len())
}

fn syntax_error_count(root: Node<'_>) -> usize {
    let count = syntax_error_nodes(root);
    if count == 0 && root.has_error() {
        1
    } else {
        count
    }
}

fn syntax_error_nodes(node: Node<'_>) -> usize {
    let own = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    own + node
        .children(&mut cursor)
        .map(syntax_error_nodes)
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_sources_require_a_version_or_ref() {
        assert!(is_floating_source(
            "git::https://github.com/acme/network.git",
            false
        ));
        assert!(!is_floating_source(
            "git::https://github.com/acme/network.git?ref=v1.2.3",
            false
        ));
        assert!(is_floating_source("terraform-aws-modules/vpc/aws", false));
        assert!(!is_floating_source("terraform-aws-modules/vpc/aws", true));
    }

    #[test]
    fn malformed_hcl_reports_parser_coverage_loss() {
        let parsed = collect("resource \"broken\" {", Language::Terraform).unwrap();
        assert!(parsed.parse_errors > 0);
    }
}
