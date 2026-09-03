use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::{
    analyzer::analyze_file,
    model::{FileAnalysis, Language},
};

const ALWAYS_IGNORED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".terraform",
    ".terragrunt-cache",
    "coverage",
    "fixtures",
    "__fixtures__",
    "vendor",
];

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub include_declarations: bool,
    pub respect_ignores: bool,
    pub max_file_bytes: u64,
    pub threads: usize,
}

pub fn scan(root: &Path, options: &ScanOptions) -> Result<Vec<FileAnalysis>> {
    if root.is_file() {
        return scan_one(
            root,
            root.parent().unwrap_or_else(|| Path::new(".")),
            options,
        )
        .map(|analysis| analysis.into_iter().collect());
    }

    let root = Arc::new(root.to_path_buf());
    let options = Arc::new(options.clone());
    let analyses = Arc::new(Mutex::new(Vec::new()));
    let first_error = Arc::new(Mutex::new(None));

    let mut builder = WalkBuilder::new(root.as_ref());
    builder
        .hidden(options.respect_ignores)
        .git_ignore(options.respect_ignores)
        .git_global(options.respect_ignores)
        .git_exclude(options.respect_ignores)
        .ignore(options.respect_ignores)
        .filter_entry(should_descend);
    if options.threads > 0 {
        builder.threads(options.threads);
    }

    builder.build_parallel().run(|| {
        let root = Arc::clone(&root);
        let options = Arc::clone(&options);
        let analyses = Arc::clone(&analyses);
        let first_error = Arc::clone(&first_error);

        Box::new(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    remember_error(&first_error, anyhow::anyhow!(error));
                    return WalkState::Continue;
                }
            };

            let path = entry.path();
            let is_file = matches!(entry.file_type(), Some(kind) if kind.is_file());
            if !is_file || !is_source(path, options.include_declarations) {
                return WalkState::Continue;
            }

            match scan_one(path, &root, &options) {
                Ok(Some(analysis)) => analyses
                    .lock()
                    .expect("analysis lock poisoned")
                    .push(analysis),
                Ok(None) => {}
                Err(error) => remember_error(&first_error, error),
            }
            WalkState::Continue
        })
    });

    if let Some(error) = first_error.lock().expect("error lock poisoned").take() {
        return Err(error);
    }

    let mut analyses = Arc::try_unwrap(analyses)
        .expect("parallel walker retained analysis state")
        .into_inner()
        .expect("analysis lock poisoned");
    analyses.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(analyses)
}

/// Scan an explicit set of files while retaining paths relative to the repository root.
/// Missing and unsupported paths are ignored because editor hooks can observe deletes.
pub fn scan_files(
    root: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
    options: &ScanOptions,
) -> Result<Vec<FileAnalysis>> {
    let mut unique = BTreeSet::new();
    for path in paths {
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        if !path.is_file() || path.is_symlink() {
            continue;
        }
        let path = path
            .canonicalize()
            .with_context(|| format!("cannot resolve '{}'", path.display()))?;
        if path.starts_with(root) {
            unique.insert(path);
        }
    }

    let mut analyses = Vec::with_capacity(unique.len());
    for path in unique {
        if let Some(analysis) = scan_one(&path, root, options)? {
            analyses.push(analysis);
        }
    }
    Ok(analyses)
}

fn scan_one(path: &Path, root: &Path, options: &ScanOptions) -> Result<Option<FileAnalysis>> {
    if !is_source(path, options.include_declarations) {
        return Ok(None);
    }
    let metadata = path
        .metadata()
        .with_context(|| format!("cannot read metadata for '{}'", path.display()))?;
    if metadata.len() > options.max_file_bytes {
        return Ok(None);
    }
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read '{}' as UTF-8", path.display()))?;
    Ok(Some(analyze_file(path, root, source, metadata.len())?))
}

fn remember_error(slot: &Mutex<Option<anyhow::Error>>, error: anyhow::Error) {
    let mut slot = slot.lock().expect("error lock poisoned");
    if slot.is_none() {
        *slot = Some(error);
    }
}

fn should_descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !entry.file_type().is_some_and(|kind| kind.is_dir())
        || !ALWAYS_IGNORED.iter().any(|ignored| name == *ignored)
}

fn is_source(path: &Path, include_declarations: bool) -> bool {
    match Language::from_path(path) {
        Some(Language::TypeScript) => {
            include_declarations
                || !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".d.ts"))
        }
        Some(Language::Rust) => true,
        Some(Language::Terraform | Language::Terragrunt) => true,
        None => false,
    }
}

#[allow(dead_code)]
fn _assert_send_sync(_: PathBuf) {}
