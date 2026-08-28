use std::collections::HashSet;

use crate::model::{Category, DependencyKind, Finding};

use super::{percentile, strongly_connected_components, ModuleGraph};

const MODULE_FANOUT_MINIMUM: usize = 12;
const COUPLING_HUB_MINIMUM: usize = 8;
const BARREL_CHAIN_LAYERS: usize = 3;
const STABLE_MODULE_INCOMING: usize = 8;
const VOLATILE_MODULE_OUTGOING: usize = 8;

pub(super) fn evaluate(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let mut findings = dependency_cycles(graph);
    findings.extend(module_fanout(graph));
    findings.extend(coupling_hubs(graph));
    findings.extend(barrel_mazes(graph));
    findings.extend(unstable_dependencies(graph));
    findings
}

fn unstable_dependencies(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let incoming = incoming_degrees(&graph.adjacency);
    let outgoing = graph.adjacency.iter().map(Vec::len).collect::<Vec<_>>();
    let mut findings = Vec::new();
    for source in 0..graph.nodes.len() {
        if incoming[source] < STABLE_MODULE_INCOMING
            || instability(incoming[source], outgoing[source]) > 0.25
        {
            continue;
        }
        for &target in &graph.adjacency[source] {
            if outgoing[target] >= VOLATILE_MODULE_OUTGOING
                && incoming[target] <= 1
                && instability(incoming[target], outgoing[target]) >= 0.8
            {
                findings.push(unstable_dependency_finding(
                    graph,
                    source,
                    target,
                    incoming[source],
                    outgoing[target],
                ));
            }
        }
    }
    findings
}

fn instability(incoming: usize, outgoing: usize) -> f64 {
    let total = incoming + outgoing;
    if total == 0 {
        0.0
    } else {
        outgoing as f64 / total as f64
    }
}

fn unstable_dependency_finding(
    graph: &ModuleGraph<'_>,
    source: usize,
    target: usize,
    source_incoming: usize,
    target_outgoing: usize,
) -> Finding {
    let source_file = graph.nodes[source].file;
    let target_path = &graph.nodes[target].file.display_path;
    Finding::new(
        "unstable-dependency",
        Category::Architecture,
        (5.0 + source_incoming as f64 / 8.0 + target_outgoing as f64 / 8.0).min(11.0),
        (source_file.display_path.clone(), 1),
        (
            "A stable module depends on a much more volatile module",
            format!(
                "used by {source_incoming} modules, but depends on `{target_path}` with {target_outgoing} outgoing dependencies"
            ),
        ),
    )
}

fn dependency_cycles(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    strongly_connected_components(&graph.adjacency)
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|component| cycle_finding(graph, &component, "dependency-cycle"))
        .collect()
}

fn cycle_finding(graph: &ModuleGraph<'_>, component: &[usize], rule: &'static str) -> Finding {
    let mut paths = component
        .iter()
        .map(|&node| graph.nodes[node].file.display_path.as_str())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    let representative = paths[0];
    let members = component.iter().copied().collect::<HashSet<_>>();
    let line = component
        .iter()
        .flat_map(|&source| &graph.edges[source])
        .find(|edge| members.contains(&edge.target))
        .map_or(1, |edge| edge.line);
    Finding::new(
        rule,
        Category::Architecture,
        (7.0 + component.len() as f64).min(14.0),
        (representative.to_owned(), line),
        (
            "Modules depend on each other circularly",
            format!(
                "{} modules in one cycle: {}",
                component.len(),
                abbreviated_paths(&paths)
            ),
        ),
    )
}

fn module_fanout(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let degrees = graph.adjacency.iter().map(Vec::len).collect::<Vec<_>>();
    let threshold = MODULE_FANOUT_MINIMUM.max(percentile(&degrees, 95));
    let mut findings = Vec::new();
    for (node, degree) in degrees.iter().enumerate() {
        if *degree < threshold {
            continue;
        }
        let file = graph.nodes[node].file;
        findings.push(Finding::new(
            "module-fanout",
            Category::Architecture,
            (4.0 + (*degree - threshold) as f64 * 0.5).min(10.0),
            (file.display_path.clone(), 1),
            (
                "This module knows about too many internal modules",
                format!("{degree} direct internal dependencies; adaptive threshold is {threshold}"),
            ),
        ));
    }
    findings
}

fn coupling_hubs(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let incoming = incoming_degrees(&graph.adjacency);
    let outgoing = graph.adjacency.iter().map(Vec::len).collect::<Vec<_>>();
    let incoming_threshold = COUPLING_HUB_MINIMUM.max(percentile(&incoming, 90));
    let outgoing_threshold = COUPLING_HUB_MINIMUM.max(percentile(&outgoing, 90));
    (0..graph.nodes.len())
        .filter(|&node| {
            incoming[node] >= incoming_threshold && outgoing[node] >= outgoing_threshold
        })
        .map(|node| hub_finding(graph, node, incoming[node], outgoing[node]))
        .collect()
}

fn hub_finding(graph: &ModuleGraph<'_>, node: usize, incoming: usize, outgoing: usize) -> Finding {
    let file = graph.nodes[node].file;
    Finding::new(
        "coupling-hub",
        Category::Architecture,
        (5.0 + (incoming + outgoing) as f64 / 8.0).min(12.0),
        (file.display_path.clone(), 1),
        (
            "This module is a high-cost dependency junction",
            format!("{incoming} incoming and {outgoing} outgoing internal dependencies"),
        ),
    )
}

fn incoming_degrees(adjacency: &[Vec<usize>]) -> Vec<usize> {
    let mut incoming = vec![0; adjacency.len()];
    for targets in adjacency {
        for &target in targets {
            incoming[target] += 1;
        }
    }
    incoming
}

fn barrel_mazes(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let barrels = (0..graph.nodes.len())
        .map(|node| is_barrel(graph, node))
        .collect::<Vec<_>>();
    let incoming = barrel_incoming(graph, &barrels);
    let mut findings = Vec::new();
    for start in 0..graph.nodes.len() {
        if !barrels[start] || incoming[start] > 0 {
            continue;
        }
        let path = longest_barrel_path(graph, &barrels, start, &mut HashSet::new());
        let layers = path.len().saturating_sub(1);
        if layers >= BARREL_CHAIN_LAYERS {
            findings.push(barrel_finding(graph, &path, layers));
        }
    }
    findings
}

fn is_barrel(graph: &ModuleGraph<'_>, node: usize) -> bool {
    let reexports = graph.nodes[node]
        .file
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind == DependencyKind::ReExport)
        .count();
    reexports > 0 && reexports == graph.nodes[node].file.top_level_statements
}

fn barrel_incoming(graph: &ModuleGraph<'_>, barrels: &[bool]) -> Vec<usize> {
    let mut incoming = vec![0; graph.nodes.len()];
    for source in 0..graph.nodes.len() {
        if !barrels[source] {
            continue;
        }
        for edge in &graph.edges[source] {
            if edge.kind == DependencyKind::ReExport && barrels[edge.target] {
                incoming[edge.target] += 1;
            }
        }
    }
    incoming
}

fn longest_barrel_path(
    graph: &ModuleGraph<'_>,
    barrels: &[bool],
    node: usize,
    visiting: &mut HashSet<usize>,
) -> Vec<usize> {
    if !visiting.insert(node) {
        return vec![node];
    }
    let mut best = vec![node];
    for edge in &graph.edges[node] {
        if edge.kind != DependencyKind::ReExport {
            continue;
        }
        let mut candidate = vec![node];
        if barrels[edge.target] {
            candidate.extend(longest_barrel_path(graph, barrels, edge.target, visiting));
        } else {
            candidate.push(edge.target);
        }
        if candidate.len() > best.len() {
            best = candidate;
        }
    }
    visiting.remove(&node);
    best
}

fn barrel_finding(graph: &ModuleGraph<'_>, path: &[usize], layers: usize) -> Finding {
    let names = path
        .iter()
        .map(|&node| graph.nodes[node].file.display_path.as_str())
        .collect::<Vec<_>>();
    let file = graph.nodes[path[0]].file;
    Finding::new(
        "barrel-maze",
        Category::Architecture,
        (4.0 + (layers - BARREL_CHAIN_LAYERS) as f64).min(9.0),
        (file.display_path.clone(), 1),
        (
            "Re-export layers obscure the implementation location",
            format!("{layers} re-export layers: {}", abbreviated_paths(&names)),
        ),
    )
}

fn abbreviated_paths(paths: &[&str]) -> String {
    let shown = paths
        .iter()
        .take(5)
        .copied()
        .collect::<Vec<_>>()
        .join(" -> ");
    if paths.len() > 5 {
        format!("{shown} -> …")
    } else {
        shown
    }
}
