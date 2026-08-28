mod architecture;
mod duplicates;
mod structure;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::{DependencyKind, FileAnalysis, Finding, Language, RepositoryMetrics};

pub struct RepositoryAnalysis {
    pub findings: Vec<Finding>,
    pub metrics: RepositoryMetrics,
}

pub fn analyze(root: &Path, files: &[FileAnalysis]) -> RepositoryAnalysis {
    let graph = ModuleGraph::build(files);
    let mut findings = architecture::evaluate(&graph);
    findings.extend(structure::evaluate(&graph));
    findings.extend(workspace::evaluate(root, &graph));
    findings.extend(duplicates::evaluate(files));
    let metrics = RepositoryMetrics {
        modules: graph.nodes.len(),
        directories: structure::directory_count(&graph),
        internal_dependencies: graph.edges.iter().map(Vec::len).sum(),
        unresolved_relative_dependencies: graph.unresolved_relative_dependencies,
    };
    RepositoryAnalysis { findings, metrics }
}

pub(super) struct ModuleGraph<'a> {
    pub nodes: Vec<ModuleNode<'a>>,
    pub edges: Vec<Vec<DependencyEdge>>,
    pub adjacency: Vec<Vec<usize>>,
    unresolved_relative_dependencies: usize,
}

pub(super) struct ModuleNode<'a> {
    pub file: &'a FileAnalysis,
    pub directory: String,
}

#[derive(Clone, Copy)]
pub(super) struct DependencyEdge {
    pub target: usize,
    pub line: usize,
    pub kind: DependencyKind,
}

impl<'a> ModuleGraph<'a> {
    fn build(files: &'a [FileAnalysis]) -> Self {
        let index = files
            .iter()
            .enumerate()
            .map(|(position, file)| (file.display_path.clone(), position))
            .collect::<HashMap<_, _>>();
        let nodes = files
            .iter()
            .map(|file| ModuleNode {
                file,
                directory: parent_path(&file.display_path).to_owned(),
            })
            .collect::<Vec<_>>();
        let mut unresolved_relative_dependencies = 0;
        let edges = files
            .iter()
            .enumerate()
            .map(|(source, file)| {
                resolve_dependencies(source, file, &index, &mut unresolved_relative_dependencies)
            })
            .collect::<Vec<_>>();
        let adjacency = edges
            .iter()
            .map(|dependencies| {
                let mut targets = dependencies
                    .iter()
                    .map(|dependency| dependency.target)
                    .collect::<Vec<_>>();
                targets.sort_unstable();
                targets.dedup();
                targets
            })
            .collect();
        Self {
            nodes,
            edges,
            adjacency,
            unresolved_relative_dependencies,
        }
    }
}

fn resolve_dependencies(
    source: usize,
    file: &FileAnalysis,
    index: &HashMap<String, usize>,
    unresolved_relative_dependencies: &mut usize,
) -> Vec<DependencyEdge> {
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for dependency in &file.dependencies {
        let targets = match file.language {
            Language::TypeScript => {
                resolve_typescript(&file.display_path, &dependency.specifier, index)
                    .into_iter()
                    .collect()
            }
            Language::Rust => resolve_rust(&file.display_path, &dependency.specifier, index),
            Language::Terraform | Language::Terragrunt => {
                resolve_hcl(&file.display_path, &dependency.specifier, index)
            }
        };
        if targets.is_empty() {
            if matches!(
                file.language,
                Language::TypeScript | Language::Terraform | Language::Terragrunt
            ) && dependency.specifier.starts_with('.')
            {
                *unresolved_relative_dependencies += 1;
            }
            continue;
        }
        for target in targets {
            if target != source && seen.insert((target, dependency.kind)) {
                edges.push(DependencyEdge {
                    target,
                    line: dependency.line,
                    kind: dependency.kind,
                });
            }
        }
    }
    edges
}

fn resolve_typescript(
    source: &str,
    specifier: &str,
    index: &HashMap<String, usize>,
) -> Option<usize> {
    let specifier = specifier.split(['?', '#']).next().unwrap_or(specifier);
    let mut bases = Vec::new();
    if specifier.starts_with('.') {
        bases.push(join_path(parent_path(source), specifier));
    } else if let Some(tail) = specifier
        .strip_prefix("@/")
        .or_else(|| specifier.strip_prefix("~/"))
    {
        bases.push(join_path("src", tail));
        bases.push(normalize_path(tail));
    } else if specifier.starts_with("src/") {
        bases.push(normalize_path(specifier));
    }
    bases
        .into_iter()
        .find_map(|base| resolve_typescript_base(&base, index))
}

fn resolve_typescript_base(base: &str, index: &HashMap<String, usize>) -> Option<usize> {
    let mut candidates = vec![base.to_owned()];
    let stem = strip_javascript_extension(base).unwrap_or(base);
    candidates.extend([
        format!("{stem}.ts"),
        format!("{stem}.tsx"),
        format!("{stem}.d.ts"),
        format!("{stem}/index.ts"),
        format!("{stem}/index.tsx"),
    ]);
    candidates
        .into_iter()
        .find_map(|candidate| index.get(&candidate).copied())
}

fn strip_javascript_extension(path: &str) -> Option<&str> {
    [".js", ".jsx", ".mjs", ".cjs"]
        .into_iter()
        .find_map(|extension| path.strip_suffix(extension))
}

fn resolve_hcl(source: &str, specifier: &str, index: &HashMap<String, usize>) -> Vec<usize> {
    let specifier = specifier
        .strip_prefix("file://")
        .unwrap_or(specifier)
        .split(['?', '#'])
        .next()
        .unwrap_or(specifier);
    let base = join_path(parent_path(source), specifier);
    let preferred = [
        base.clone(),
        format!("{base}/terragrunt.hcl"),
        format!("{base}/main.tf"),
        format!("{base}/root.hcl"),
    ];
    if let Some(target) = preferred
        .iter()
        .find_map(|candidate| index.get(candidate).copied())
    {
        return vec![target];
    }
    let mut module_files = index
        .iter()
        .filter(|(path, _)| {
            parent_path(path) == base && (path.ends_with(".tf") || path.ends_with(".hcl"))
        })
        .map(|(path, target)| (path, *target))
        .collect::<Vec<_>>();
    module_files.sort_unstable_by(|left, right| left.0.cmp(right.0));
    module_files
        .first()
        .map_or_else(Vec::new, |(_, target)| vec![*target])
}

fn resolve_rust(source: &str, specifier: &str, index: &HashMap<String, usize>) -> Vec<usize> {
    let layout = RustModuleLayout::from_source(source);
    let paths = if let Some(module) = specifier.strip_prefix("mod:") {
        vec![format!("self::{module}")]
    } else {
        expand_rust_use(specifier)
    };
    let mut targets = paths
        .iter()
        .filter_map(|path| resolve_rust_path(&layout, path, index))
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    targets
}

struct RustModuleLayout {
    source_root: String,
    module: Vec<String>,
}

impl RustModuleLayout {
    fn from_source(source: &str) -> Self {
        let parts = source.split('/').collect::<Vec<_>>();
        let source_index = parts.iter().rposition(|part| *part == "src");
        let root_end = source_index.map_or(0, |index| index + 1);
        let source_root = parts[..root_end].join("/");
        let relative = &parts[root_end..];
        let file = relative.last().copied().unwrap_or_default();
        let mut module = relative[..relative.len().saturating_sub(1)]
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>();
        if !matches!(file, "lib.rs" | "main.rs" | "mod.rs") {
            module.push(file.strip_suffix(".rs").unwrap_or(file).to_owned());
        }
        Self {
            source_root,
            module,
        }
    }
}

fn resolve_rust_path(
    layout: &RustModuleLayout,
    path: &str,
    index: &HashMap<String, usize>,
) -> Option<usize> {
    let mut segments = path
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != "*")
        .map(|segment| segment.strip_prefix("r#").unwrap_or(segment).to_owned())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }
    let mut module = match segments[0].as_str() {
        "crate" => {
            segments.remove(0);
            Vec::new()
        }
        "self" => {
            segments.remove(0);
            layout.module.clone()
        }
        "super" => super_base(&mut segments, &layout.module),
        _ => Vec::new(),
    };
    module.extend(segments);
    for length in (1..=module.len()).rev() {
        if let Some(target) = rust_module_candidate(&layout.source_root, &module[..length], index) {
            return Some(target);
        }
    }
    None
}

fn super_base(segments: &mut Vec<String>, current: &[String]) -> Vec<String> {
    let mut base = current.to_vec();
    while segments.first().is_some_and(|segment| segment == "super") {
        segments.remove(0);
        base.pop();
    }
    base
}

fn rust_module_candidate(
    source_root: &str,
    module: &[String],
    index: &HashMap<String, usize>,
) -> Option<usize> {
    let base = join_path(source_root, &module.join("/"));
    [format!("{base}.rs"), format!("{base}/mod.rs")]
        .into_iter()
        .find_map(|candidate| index.get(&candidate).copied())
}

fn expand_rust_use(specifier: &str) -> Vec<String> {
    let specifier = specifier.split(" as ").next().unwrap_or(specifier).trim();
    let Some(open) = specifier.find('{') else {
        return vec![specifier.trim_end_matches("::").to_owned()];
    };
    let Some(close) = matching_brace(specifier, open) else {
        return vec![specifier.to_owned()];
    };
    let prefix = specifier[..open].trim_end_matches("::");
    split_rust_use_items(&specifier[open + 1..close])
        .into_iter()
        .flat_map(|item| {
            if item == "self" {
                vec![prefix.to_owned()]
            } else {
                expand_rust_use(&format!("{prefix}::{item}"))
            }
        })
        .collect()
}

fn matching_brace(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (offset, character) in value[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_rust_use_items(value: &str) -> Vec<&str> {
    let mut depth = 0;
    let mut start = 0;
    let mut items = Vec::new();
    for (index, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                items.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    items.push(value[start..].trim());
    items.into_iter().filter(|item| !item.is_empty()).collect()
}

pub(super) fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

pub(super) fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

pub(super) fn join_path(parent: &str, child: &str) -> String {
    normalize_path(&format!("{parent}/{child}"))
}

pub(super) fn normalize_path(path: &str) -> String {
    let mut parts = Vec::new();
    let normalized = path.replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

pub(super) fn percentile(values: &[usize], percentile: usize) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

pub(super) fn strongly_connected_components(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let order = finishing_order(adjacency);
    let reverse = reverse_adjacency(adjacency);
    collect_components(&order, &reverse)
}

fn reverse_adjacency(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut reverse = vec![Vec::new(); adjacency.len()];
    for (source, targets) in adjacency.iter().enumerate() {
        for &target in targets {
            reverse[target].push(source);
        }
    }
    reverse
}

fn collect_components(order: &[usize], reverse: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut assigned = vec![false; reverse.len()];
    let mut components = Vec::new();
    for &start in order.iter().rev() {
        if assigned[start] {
            continue;
        }
        components.push(collect_component(start, reverse, &mut assigned));
    }
    components
}

fn collect_component(start: usize, reverse: &[Vec<usize>], assigned: &mut [bool]) -> Vec<usize> {
    let mut component = Vec::new();
    let mut stack = vec![start];
    assigned[start] = true;
    while let Some(node) = stack.pop() {
        component.push(node);
        for &next in &reverse[node] {
            if !assigned[next] {
                assigned[next] = true;
                stack.push(next);
            }
        }
    }
    component
}

fn finishing_order(adjacency: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; adjacency.len()];
    let mut order = Vec::with_capacity(adjacency.len());
    for start in 0..adjacency.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, next_index)) = stack.last_mut() {
            if let Some(&next) = adjacency[*node].get(*next_index) {
                *next_index += 1;
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_resolution_understands_relative_and_common_root_aliases() {
        let index = HashMap::from([
            ("src/domain/order.ts".to_owned(), 0),
            ("src/shared/index.ts".to_owned(), 1),
        ]);
        assert_eq!(
            resolve_typescript("src/api/handler.ts", "../domain/order", &index),
            Some(0)
        );
        assert_eq!(
            resolve_typescript("src/api/handler.ts", "@/shared", &index),
            Some(1)
        );
    }

    #[test]
    fn components_are_found_without_recursive_graph_walks() {
        let components = strongly_connected_components(&[vec![1], vec![0, 2], vec![]]);
        assert!(components.iter().any(|component| component.len() == 2));
        assert!(components.iter().any(|component| component == &[2]));
    }

    #[test]
    fn hcl_resolution_understands_module_and_terragrunt_directories() {
        let index = HashMap::from([
            ("modules/network/main.tf".to_owned(), 0),
            ("live/shared/terragrunt.hcl".to_owned(), 1),
        ]);
        assert_eq!(
            resolve_hcl("infra/main.tf", "../modules/network", &index),
            vec![0]
        );
        assert_eq!(
            resolve_hcl("live/app/terragrunt.hcl", "../shared", &index),
            vec![1]
        );
    }
}
