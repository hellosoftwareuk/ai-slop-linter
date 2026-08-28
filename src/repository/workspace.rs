use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{Category, Finding, Language};

use super::ModuleGraph;

struct Package {
    root: PathBuf,
    name: String,
    exports: Option<Vec<String>>,
}

pub(super) fn evaluate(_root: &Path, graph: &ModuleGraph<'_>) -> Vec<Finding> {
    let packages = discover_packages(graph);
    if packages.len() < 2 {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let mut seen = HashSet::new();
    for node in &graph.nodes {
        if node.file.language != Language::TypeScript {
            continue;
        }
        for dependency in &node.file.dependencies {
            let Some(package) = package_for_specifier(&dependency.specifier, &packages) else {
                continue;
            };
            if node.file.path.starts_with(&package.root)
                || package_allows(package, &dependency.specifier)
                || !seen.insert((
                    node.file.display_path.as_str(),
                    dependency.specifier.as_str(),
                ))
            {
                continue;
            }
            findings.push(Finding::new(
                "workspace-boundary-bypass",
                Category::Architecture,
                6.0,
                (node.file.display_path.clone(), dependency.line),
                (
                    "This import bypasses another workspace package's public API",
                    format!(
                        "`{}` is not declared by the `exports` map for `{}`",
                        dependency.specifier, package.name
                    ),
                ),
            ));
        }
    }
    findings
}

fn discover_packages(graph: &ModuleGraph<'_>) -> Vec<Package> {
    let manifests = graph
        .nodes
        .iter()
        .flat_map(|node| {
            node.file
                .path
                .parent()
                .into_iter()
                .flat_map(Path::ancestors)
                .take(8)
                .map(|directory| directory.join("package.json"))
        })
        .filter(|manifest| manifest.is_file())
        .collect::<BTreeSet<_>>();
    manifests
        .into_iter()
        .filter_map(|manifest| read_package(&manifest))
        .collect()
}

fn read_package(manifest: &Path) -> Option<Package> {
    let source = std::fs::read_to_string(manifest).ok()?;
    let value = serde_json::from_str::<Value>(&source).ok()?;
    let name = value.get("name")?.as_str()?.to_owned();
    let exports = value.get("exports").map(export_keys);
    Some(Package {
        root: manifest.parent()?.to_path_buf(),
        name,
        exports,
    })
}

fn export_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Object(entries) => {
            let keys = entries
                .keys()
                .filter(|key| key.starts_with('.'))
                .cloned()
                .collect::<Vec<_>>();
            if keys.is_empty() {
                vec![".".to_owned()]
            } else {
                keys
            }
        }
        Value::Null => Vec::new(),
        _ => vec![".".to_owned()],
    }
}

fn package_for_specifier<'a>(specifier: &str, packages: &'a [Package]) -> Option<&'a Package> {
    packages
        .iter()
        .filter(|package| {
            specifier == package.name
                || specifier
                    .strip_prefix(&package.name)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|package| package.name.len())
}

fn package_allows(package: &Package, specifier: &str) -> bool {
    let Some(exports) = &package.exports else {
        return true;
    };
    let suffix = specifier.strip_prefix(&package.name).unwrap_or_default();
    let requested = if suffix.is_empty() {
        ".".to_owned()
    } else {
        format!(".{suffix}")
    };
    exports
        .iter()
        .any(|export| export_matches(export, &requested))
}

fn export_matches(pattern: &str, requested: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return pattern == requested;
    };
    requested.starts_with(prefix)
        && requested.ends_with(suffix)
        && requested.len() >= prefix.len() + suffix.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_patterns_allow_only_declared_subpaths() {
        assert!(export_matches("./features/*", "./features/orders"));
        assert!(!export_matches("./features/*", "./src/internal"));
    }
}
