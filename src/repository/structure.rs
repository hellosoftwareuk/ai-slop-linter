use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::model::{Category, DependencyKind, Finding};

use super::{file_name, parent_path, percentile, strongly_connected_components, ModuleGraph};

const CROWDED_FOLDER_MINIMUM: usize = 25;
const WIDE_FOLDER_MINIMUM: usize = 12;
const TRANSIT_FOLDER_CHAIN: usize = 3;
const FOLDER_HUB_MINIMUM: usize = 8;
const MISPLACED_DEPENDENCIES: usize = 5;
const CATCH_ALL_MODULES: usize = 12;
const CATCH_ALL_EXTERNAL_FOLDERS: usize = 5;

#[derive(Default)]
struct DirectoryData {
    direct_files: usize,
    child_directories: BTreeSet<String>,
}

struct FolderGraph {
    names: Vec<String>,
    adjacency: Vec<Vec<usize>>,
}

pub(super) fn evaluate(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let directories = build_directories(graph);
    let mut findings = crowded_folders(&directories);
    findings.extend(wide_folders(&directories));
    findings.extend(deep_folder_chains(&directories));
    findings.extend(wrapper_directories(graph, &directories));
    let folder_graph = build_folder_graph(graph);
    findings.extend(folder_dependency_cycles(&folder_graph));
    findings.extend(folder_coupling_hubs(&folder_graph));
    findings.extend(misplaced_modules(graph));
    findings.extend(catch_all_folders(graph, &directories));
    findings
}

fn wrapper_directories(
    graph: &ModuleGraph<'_>,
    directories: &BTreeMap<String, DirectoryData>,
) -> Vec<Finding> {
    directories
        .iter()
        .filter(|(_, directory_info)| {
            directory_info.direct_files == 1 && directory_info.child_directories.len() == 1
        })
        .filter_map(|(directory, directory_info)| {
            let node = graph
                .nodes
                .iter()
                .position(|node| node.directory == *directory)?;
            let name = file_name(&graph.nodes[node].file.display_path);
            if !matches!(name, "index.ts" | "index.tsx" | "mod.rs") {
                return None;
            }
            let reexports = graph.nodes[node]
                .file
                .dependencies
                .iter()
                .filter(|dependency| dependency.kind == DependencyKind::ReExport)
                .count();
            if reexports == 0 || reexports != graph.nodes[node].file.top_level_statements {
                return None;
            }
            let child = directory_info.child_directories.first()?;
            let targets = graph.edges[node]
                .iter()
                .filter(|edge| edge.kind == DependencyKind::ReExport)
                .collect::<Vec<_>>();
            if targets.is_empty()
                || !targets
                    .iter()
                    .all(|edge| is_within(&graph.nodes[edge.target].directory, child))
            {
                return None;
            }
            Some(Finding::new(
                "wrapper-directory",
                Category::Structure,
                4.0,
                (directory.clone(), 1),
                (
                    "This directory adds a re-export stop without adding organization",
                    format!("one entrypoint only forwards into its sole child `{child}`"),
                ),
            ))
        })
        .collect()
}

pub(super) fn directory_count(graph: &ModuleGraph<'_>) -> usize {
    build_directories(graph)
        .keys()
        .filter(|directory| !directory.is_empty())
        .count()
}

fn build_directories(graph: &ModuleGraph<'_>) -> BTreeMap<String, DirectoryData> {
    let mut directories = BTreeMap::<String, DirectoryData>::new();
    for node in &graph.nodes {
        directories
            .entry(node.directory.clone())
            .or_default()
            .direct_files += 1;
        let mut child = node.directory.as_str();
        while !child.is_empty() {
            let parent = parent_path(child);
            directories
                .entry(parent.to_owned())
                .or_default()
                .child_directories
                .insert(child.to_owned());
            directories.entry(child.to_owned()).or_default();
            child = parent;
        }
    }
    directories
}

fn crowded_folders(directories: &BTreeMap<String, DirectoryData>) -> Vec<Finding> {
    let counts = directories
        .values()
        .map(|directory| directory.direct_files)
        .filter(|count| *count > 0)
        .collect::<Vec<_>>();
    let threshold = CROWDED_FOLDER_MINIMUM.max(percentile(&counts, 95));
    directories
        .iter()
        .filter(|(_, directory_info)| directory_info.direct_files >= threshold)
        .map(|(path, directory_info)| {
            folder_size_finding(
                "crowded-folder",
                path,
                directory_info.direct_files,
                threshold,
                "source files",
            )
        })
        .collect()
}

fn wide_folders(directories: &BTreeMap<String, DirectoryData>) -> Vec<Finding> {
    let counts = directories
        .values()
        .map(|directory| directory.child_directories.len())
        .filter(|count| *count > 0)
        .collect::<Vec<_>>();
    let threshold = WIDE_FOLDER_MINIMUM.max(percentile(&counts, 95));
    directories
        .iter()
        .filter(|(_, directory_info)| directory_info.child_directories.len() >= threshold)
        .map(|(path, directory_info)| {
            folder_size_finding(
                "wide-folder",
                path,
                directory_info.child_directories.len(),
                threshold,
                "immediate subdirectories",
            )
        })
        .collect()
}

fn folder_size_finding(
    rule: &'static str,
    path: &str,
    count: usize,
    threshold: usize,
    unit: &str,
) -> Finding {
    Finding::new(
        rule,
        Category::Structure,
        (4.0 + (count - threshold) as f64 * 0.25).min(10.0),
        (display_directory(path), 1),
        (
            "This directory is expensive to scan and navigate",
            format!("{count} {unit}; adaptive threshold is {threshold}"),
        ),
    )
}

fn deep_folder_chains(directories: &BTreeMap<String, DirectoryData>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in directories.keys() {
        if path_depth(path) < 2 || !is_transit_directory(path, directories) {
            continue;
        }
        let parent = parent_path(path);
        if path_depth(parent) >= 2 && is_transit_directory(parent, directories) {
            continue;
        }
        let chain = transit_chain(path, directories);
        if chain.len() >= TRANSIT_FOLDER_CHAIN {
            findings.push(transit_chain_finding(&chain));
        }
    }
    findings
}

fn is_transit_directory(path: &str, directories: &BTreeMap<String, DirectoryData>) -> bool {
    directories.get(path).is_some_and(|directory| {
        directory.direct_files == 0 && directory.child_directories.len() == 1
    })
}

fn transit_chain(start: &str, directories: &BTreeMap<String, DirectoryData>) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = start;
    while is_transit_directory(current, directories) {
        chain.push(current.to_owned());
        current = directories[current]
            .child_directories
            .first()
            .expect("transit directories have one child");
    }
    chain
}

fn transit_chain_finding(chain: &[String]) -> Finding {
    Finding::new(
        "deep-folder-chain",
        Category::Structure,
        (4.0 + (chain.len() - TRANSIT_FOLDER_CHAIN) as f64).min(9.0),
        (chain[0].clone(), 1),
        (
            "Single-child directories add navigation without grouping choices",
            format!(
                "{} transit directories: {}",
                chain.len(),
                chain.join(" -> ")
            ),
        ),
    )
}

fn build_folder_graph(graph: &ModuleGraph<'_>) -> FolderGraph {
    let names = graph
        .nodes
        .iter()
        .map(|node| node.directory.clone())
        .filter(|directory| !directory.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let index = names
        .iter()
        .enumerate()
        .map(|(position, name)| (name.as_str(), position))
        .collect::<HashMap<_, _>>();
    let mut adjacency = vec![BTreeSet::new(); names.len()];
    for (source, targets) in graph.adjacency.iter().enumerate() {
        let source_directory = graph.nodes[source].directory.as_str();
        let Some(&source_index) = index.get(source_directory) else {
            continue;
        };
        for &target in targets {
            let target_directory = graph.nodes[target].directory.as_str();
            if source_directory != target_directory {
                if let Some(&target_index) = index.get(target_directory) {
                    adjacency[source_index].insert(target_index);
                }
            }
        }
    }
    FolderGraph {
        names,
        adjacency: adjacency
            .into_iter()
            .map(|targets| targets.into_iter().collect())
            .collect(),
    }
}

fn folder_dependency_cycles(graph: &FolderGraph) -> Vec<Finding> {
    strongly_connected_components(&graph.adjacency)
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|component| {
            let mut names = component
                .iter()
                .map(|&directory| graph.names[directory].as_str())
                .collect::<Vec<_>>();
            names.sort_unstable();
            Finding::new(
                "folder-dependency-cycle",
                Category::Structure,
                (6.0 + component.len() as f64).min(13.0),
                (names[0].to_owned(), 1),
                (
                    "Directories have circular responsibilities",
                    format!(
                        "{} directories in one cycle: {}",
                        names.len(),
                        names.join(", ")
                    ),
                ),
            )
        })
        .collect()
}

fn folder_coupling_hubs(graph: &FolderGraph) -> Vec<Finding> {
    let outgoing = graph.adjacency.iter().map(Vec::len).collect::<Vec<_>>();
    let incoming = incoming_degrees(&graph.adjacency);
    let incoming_threshold = FOLDER_HUB_MINIMUM.max(percentile(&incoming, 90));
    let outgoing_threshold = FOLDER_HUB_MINIMUM.max(percentile(&outgoing, 90));
    (0..graph.names.len())
        .filter(|&directory| {
            incoming[directory] >= incoming_threshold && outgoing[directory] >= outgoing_threshold
        })
        .map(|directory| {
            Finding::new(
                "folder-coupling-hub",
                Category::Structure,
                (5.0 + (incoming[directory] + outgoing[directory]) as f64 / 8.0).min(12.0),
                (graph.names[directory].clone(), 1),
                (
                    "This directory is a high-cost structural junction",
                    format!(
                        "{} incoming and {} outgoing directory dependencies",
                        incoming[directory], outgoing[directory]
                    ),
                ),
            )
        })
        .collect()
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

fn misplaced_modules(graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for source in 0..graph.nodes.len() {
        let targets = &graph.adjacency[source];
        if targets.len() < MISPLACED_DEPENDENCIES || is_entrypoint(graph, source) {
            continue;
        }
        let counts = target_directory_counts(graph, targets);
        let same_directory = counts
            .get(graph.nodes[source].directory.as_str())
            .copied()
            .unwrap_or_default();
        let Some((dominant, count)) = counts.iter().max_by_key(|(_, count)| *count) else {
            continue;
        };
        if *dominant != graph.nodes[source].directory.as_str()
            && *count >= MISPLACED_DEPENDENCIES
            && *count * 100 >= targets.len() * 80
            && same_directory <= 1
        {
            findings.push(misplaced_finding(
                graph,
                source,
                dominant,
                *count,
                targets.len(),
            ));
        }
    }
    findings
}

fn target_directory_counts<'a>(
    graph: &'a ModuleGraph<'_>,
    targets: &[usize],
) -> HashMap<&'a str, usize> {
    let mut counts = HashMap::new();
    for &target in targets {
        *counts
            .entry(graph.nodes[target].directory.as_str())
            .or_insert(0) += 1;
    }
    counts
}

fn is_entrypoint(graph: &ModuleGraph<'_>, node: usize) -> bool {
    matches!(
        file_name(&graph.nodes[node].file.display_path),
        "index.ts" | "index.tsx" | "lib.rs" | "main.rs" | "mod.rs"
    )
}

fn misplaced_finding(
    graph: &ModuleGraph<'_>,
    source: usize,
    dominant: &str,
    dominant_count: usize,
    total: usize,
) -> Finding {
    let file = graph.nodes[source].file;
    Finding::new(
        "misplaced-module",
        Category::Structure,
        (4.0 + (dominant_count - MISPLACED_DEPENDENCIES) as f64 * 0.5).min(9.0),
        (file.display_path.clone(), 1),
        (
            "This module is more structurally connected to another directory",
            format!("{dominant_count} of {total} internal dependencies target `{dominant}`"),
        ),
    )
}

fn catch_all_folders(
    graph: &ModuleGraph<'_>,
    directories: &BTreeMap<String, DirectoryData>,
) -> Vec<Finding> {
    directories
        .keys()
        .filter(|directory| is_generic_directory(directory))
        .filter_map(|directory| catch_all_finding(graph, directory))
        .collect()
}

fn catch_all_finding(graph: &ModuleGraph<'_>, directory: &str) -> Option<Finding> {
    let inside = graph
        .nodes
        .iter()
        .map(|node| is_within(&node.directory, directory))
        .collect::<Vec<_>>();
    let modules = inside.iter().filter(|is_inside| **is_inside).count();
    if modules < CATCH_ALL_MODULES {
        return None;
    }
    let (internal, boundary, external) = folder_edge_profile(graph, &inside);
    let total = internal + boundary;
    if boundary == 0 || external.len() < CATCH_ALL_EXTERNAL_FOLDERS || internal * 4 > total {
        return None;
    }
    Some(Finding::new(
        "catch-all-folder",
        Category::Structure,
        (5.0 + modules as f64 / 8.0 + external.len() as f64 / 4.0).min(12.0),
        (directory.to_owned(), 1),
        (
            "A generic directory has become a low-cohesion dependency bucket",
            format!(
                "{modules} modules, {internal} internal edges, {boundary} boundary edges, {} external directories",
                external.len()
            ),
        ),
    ))
}

fn folder_edge_profile(
    graph: &ModuleGraph<'_>,
    inside: &[bool],
) -> (usize, usize, HashSet<String>) {
    let mut internal = 0;
    let mut boundary = 0;
    let mut external = HashSet::new();
    for (source, targets) in graph.adjacency.iter().enumerate() {
        for &target in targets {
            match (inside[source], inside[target]) {
                (true, true) => internal += 1,
                (true, false) => {
                    boundary += 1;
                    external.insert(graph.nodes[target].directory.clone());
                }
                (false, true) => {
                    boundary += 1;
                    external.insert(graph.nodes[source].directory.clone());
                }
                (false, false) => {}
            }
        }
    }
    (internal, boundary, external)
}

fn is_generic_directory(path: &str) -> bool {
    matches!(
        file_name(path).to_ascii_lowercase().as_str(),
        "common" | "helper" | "helpers" | "misc" | "shared" | "util" | "utils"
    )
}

fn is_within(path: &str, directory: &str) -> bool {
    path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn path_depth(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}

fn display_directory(path: &str) -> String {
    if path.is_empty() {
        ".".to_owned()
    } else {
        path.to_owned()
    }
}
