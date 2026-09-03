use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Instant,
};

use anyhow::{bail, Context, Result};

use crate::{
    discovery::{scan, ScanOptions},
    model::{Language, ScanReport},
    scoring::build_report,
};

pub struct RepositoryContext {
    pub repo_root: PathBuf,
    pub scan_path: PathBuf,
    scope_relative: PathBuf,
    scope_is_file: bool,
}

struct BlobEntry {
    oid: String,
    path: PathBuf,
    size: u64,
}

impl RepositoryContext {
    pub fn discover(path: &Path) -> Result<Self> {
        let scan_path = path
            .canonicalize()
            .with_context(|| format!("cannot access '{}'", path.display()))?;
        let command_root = if scan_path.is_file() {
            scan_path.parent().context("source file has no parent")?
        } else {
            &scan_path
        };
        let output = git_output(command_root, &["rev-parse", "--show-toplevel"])?;
        let repo_root = PathBuf::from(String::from_utf8(output.stdout)?.trim())
            .canonicalize()
            .context("cannot resolve Git repository root")?;
        let scope_relative = scan_path
            .strip_prefix(&repo_root)
            .with_context(|| {
                format!(
                    "'{}' is outside Git repository '{}'",
                    scan_path.display(),
                    repo_root.display()
                )
            })?
            .to_path_buf();
        let scope_is_file = scan_path.is_file();
        Ok(Self {
            repo_root,
            scan_path,
            scope_relative,
            scope_is_file,
        })
    }

    fn pathspec(&self) -> String {
        if self.scope_relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            self.scope_relative.to_string_lossy().replace('\\', "/")
        }
    }

    pub fn annotation_prefix(&self) -> String {
        let prefix = if self.scope_is_file {
            self.scope_relative
                .parent()
                .unwrap_or_else(|| Path::new(""))
        } else {
            &self.scope_relative
        };
        prefix.to_string_lossy().replace('\\', "/")
    }
}

pub fn resolve_commit(
    context: &RepositoryContext,
    revision: &str,
    required: bool,
) -> Result<Option<String>> {
    let specification = format!("{revision}^{{commit}}");
    let output = git_raw_output(
        &context.repo_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &specification,
        ],
    )?;
    if output.status.success() {
        return Ok(Some(String::from_utf8(output.stdout)?.trim().to_owned()));
    }
    if required {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "cannot resolve base revision '{revision}': {}",
            stderr.trim()
        );
    }
    Ok(None)
}

pub fn index_tree(context: &RepositoryContext) -> Result<String> {
    let output = git_output(&context.repo_root, &["write-tree"])?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub fn snapshot_report(
    context: &RepositoryContext,
    treeish: &str,
    options: &ScanOptions,
) -> Result<ScanReport> {
    let started = Instant::now();
    let directory = tempfile::tempdir().context("cannot create Git snapshot directory")?;
    fs::create_dir(directory.path().join(".git"))?;
    materialize_tree(context, treeish, directory.path(), options.max_file_bytes)?;

    let snapshot_path = directory.path().join(&context.scope_relative);
    let analyses = if context.scope_is_file {
        if snapshot_path.is_file() {
            scan(&snapshot_path, options)?
        } else {
            Vec::new()
        }
    } else {
        fs::create_dir_all(&snapshot_path)?;
        scan(&snapshot_path, options)?
    };
    Ok(build_report(&snapshot_path, analyses, started.elapsed()))
}

pub fn changed_file_count(
    context: &RepositoryContext,
    base_commit: Option<&str>,
    staged: bool,
) -> Result<usize> {
    let pathspec = context.pathspec();
    let mut paths = BTreeSet::new();
    match (base_commit, staged) {
        (Some(base), true) => collect_paths(
            git_output(
                &context.repo_root,
                &[
                    "diff",
                    "--cached",
                    "--name-only",
                    "-z",
                    base,
                    "--",
                    &pathspec,
                ],
            )?
            .stdout,
            &mut paths,
        ),
        (Some(base), false) => {
            collect_paths(
                git_output(
                    &context.repo_root,
                    &["diff", "--name-only", "-z", base, "--", &pathspec],
                )?
                .stdout,
                &mut paths,
            );
            collect_untracked(context, &pathspec, &mut paths)?;
        }
        (None, true) => collect_paths(
            git_output(
                &context.repo_root,
                &["ls-files", "--cached", "-z", "--", &pathspec],
            )?
            .stdout,
            &mut paths,
        ),
        (None, false) => collect_paths(
            git_output(
                &context.repo_root,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--",
                    &pathspec,
                ],
            )?
            .stdout,
            &mut paths,
        ),
    }
    Ok(paths.len())
}

fn collect_untracked(
    context: &RepositoryContext,
    pathspec: &str,
    paths: &mut BTreeSet<String>,
) -> Result<()> {
    let output = git_output(
        &context.repo_root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            pathspec,
        ],
    )?;
    collect_paths(output.stdout, paths);
    Ok(())
}

fn collect_paths(output: Vec<u8>, paths: &mut BTreeSet<String>) {
    for path in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        paths.insert(String::from_utf8_lossy(path).into_owned());
    }
}

fn materialize_tree(
    context: &RepositoryContext,
    treeish: &str,
    destination: &Path,
    max_file_bytes: u64,
) -> Result<()> {
    let output = git_output(
        &context.repo_root,
        &["ls-tree", "-r", "-z", "-l", "--full-tree", treeish],
    )?;
    let entries = parse_tree_entries(
        &output.stdout,
        &context.scope_relative,
        context.scope_is_file,
        max_file_bytes,
    )?;
    if entries.is_empty() {
        return Ok(());
    }
    materialize_blobs(&context.repo_root, &entries, destination)
}

fn parse_tree_entries(
    output: &[u8],
    scope: &Path,
    scope_is_file: bool,
    max_file_bytes: u64,
) -> Result<Vec<BlobEntry>> {
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            bail!("Git returned a malformed tree entry");
        };
        let metadata = std::str::from_utf8(&record[..tab])?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().context("Git tree entry has no mode")?;
        let kind = fields.next().context("Git tree entry has no type")?;
        let oid = fields.next().context("Git tree entry has no object id")?;
        let size = fields.next().context("Git tree entry has no size")?;
        if kind != "blob" || !mode.starts_with("100") {
            continue;
        }
        let size = size
            .parse::<u64>()
            .context("Git tree entry has an invalid size")?;
        let Ok(path) = std::str::from_utf8(&record[tab + 1..]) else {
            continue;
        };
        let path = PathBuf::from(path);
        if should_materialize(&path)
            && within_scan_scope(&path, scope, scope_is_file)
            && safe_relative_path(&path)
            && size <= max_file_bytes
        {
            entries.push(BlobEntry {
                oid: oid.to_owned(),
                path,
                size,
            });
        }
    }
    Ok(entries)
}

fn within_scan_scope(path: &Path, scope: &Path, scope_is_file: bool) -> bool {
    if scope.as_os_str().is_empty() {
        return true;
    }
    if scope_is_file {
        return path == scope || is_ancestor_ignore_file(path, scope);
    }
    path.starts_with(scope) || is_ancestor_ignore_file(path, scope)
}

fn is_ancestor_ignore_file(path: &Path, scope: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == ".gitignore" || name == ".ignore")
        && path
            .parent()
            .is_some_and(|parent| scope.starts_with(parent))
}

fn should_materialize(path: &Path) -> bool {
    if Language::from_path(path).is_some() {
        return true;
    }
    path.file_name().is_some_and(|name| {
        matches!(
            name.to_string_lossy().as_ref(),
            "package.json" | ".gitignore" | ".ignore"
        )
    })
}

fn safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn materialize_blobs(repo_root: &Path, entries: &[BlobEntry], destination: &Path) -> Result<()> {
    let mut child = git_command(repo_root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start `git cat-file --batch`")?;
    let mut stdin = child.stdin.take().context("cannot open Git batch input")?;
    let requests = entries
        .iter()
        .map(|entry| entry.oid.clone())
        .collect::<Vec<_>>();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        for oid in requests {
            writeln!(stdin, "{oid}")?;
        }
        Ok(())
    });

    let stdout = child
        .stdout
        .take()
        .context("cannot open Git batch output")?;
    let mut reader = BufReader::new(stdout);
    let read_result = read_blob_responses(&mut reader, entries, destination);
    if read_result.is_err() {
        let _ = child.kill();
    }
    let write_result = writer
        .join()
        .map_err(|_| anyhow::anyhow!("Git batch input thread panicked"))?;
    if let Err(error) = read_result {
        let _ = child.wait();
        return Err(error);
    }
    write_result.context("cannot write Git batch input")?;
    let status = child.wait()?;
    if !status.success() {
        bail!("`git cat-file --batch` exited with {status}");
    }
    Ok(())
}

fn read_blob_responses(
    reader: &mut impl BufRead,
    entries: &[BlobEntry],
    destination: &Path,
) -> Result<()> {
    let mut header = String::new();
    for entry in entries {
        header.clear();
        if reader.read_line(&mut header)? == 0 {
            bail!("Git batch output ended before '{}'", entry.path.display());
        }
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || fields[1] != "blob" {
            bail!(
                "Git could not read '{}': {}",
                entry.path.display(),
                header.trim()
            );
        }
        let size = fields[2]
            .parse::<usize>()
            .context("Git returned an invalid blob size")?;
        if u64::try_from(size) != Ok(entry.size) {
            bail!("Git returned the wrong size for '{}'", entry.path.display());
        }
        let mut content = vec![0; size];
        reader.read_exact(&mut content)?;
        let mut newline = [0];
        reader.read_exact(&mut newline)?;
        if newline[0] != b'\n' {
            bail!(
                "Git returned malformed content for '{}'",
                entry.path.display()
            );
        }
        let target = destination.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, content)
            .with_context(|| format!("cannot materialize '{}'", entry.path.display()))?;
    }
    Ok(())
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<Output> {
    let output = git_raw_output(root, arguments)?;
    if output.status.success() {
        return Ok(output);
    }
    let command = arguments.join(" ");
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("`git {command}` failed: {}", stderr.trim());
}

fn git_raw_output(root: &Path, arguments: &[&str]) -> Result<Output> {
    git_command(root)
        .args(arguments)
        .output()
        .with_context(|| format!("cannot run Git in '{}'", root.display()))
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).env("GIT_OPTIONAL_LOCKS", "0");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_parser_keeps_only_regular_relevant_files() -> Result<()> {
        let output = b"100644 blob abc 12\tsrc/app.ts\0\
100644 blob def 12\tREADME.md\0\
100644 blob ghi 12\tpackage.json\0\
120000 blob jkl 12\tsrc/link.rs\0";
        let entries = parse_tree_entries(output, Path::new(""), false, 2_000_000)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, PathBuf::from("src/app.ts"));
        assert_eq!(entries[1].path, PathBuf::from("package.json"));
        Ok(())
    }

    #[test]
    fn tree_parser_limits_blobs_to_the_requested_scope_and_parent_ignores() -> Result<()> {
        let output = b"100644 blob aaa 12\t.gitignore\0\
100644 blob bbb 12\tpackages/.gitignore\0\
100644 blob ccc 12\tpackages/web/app.ts\0\
100644 blob ddd 12\tpackages/api/app.ts\0";
        let entries = parse_tree_entries(output, Path::new("packages/web"), false, 2_000_000)?;
        let paths = entries
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                PathBuf::from(".gitignore"),
                PathBuf::from("packages/.gitignore"),
                PathBuf::from("packages/web/app.ts"),
            ]
        );
        Ok(())
    }

    #[test]
    fn tree_parser_rejects_oversized_blobs_before_materializing_them() -> Result<()> {
        let output = b"100644 blob aaa 2000001\tsrc/large.ts\0\
100644 blob bbb 20\tsrc/small.ts\0";
        let entries = parse_tree_entries(output, Path::new(""), false, 2_000_000)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("src/small.ts"));
        Ok(())
    }
}
