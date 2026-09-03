use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    discovery::{scan, scan_files, ScanOptions},
    model::{Diagnostic, FileAnalysis, Finding, ScanReport},
    scoring::build_report,
};

mod install;

pub use install::install;

const MAX_HOOK_FINDINGS: usize = 20;

#[derive(Debug, Deserialize)]
struct HookInput {
    hook_event_name: String,
    cwd: PathBuf,
    session_id: String,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    tool_input: Value,
    #[serde(default)]
    tool_response: Value,
    #[serde(default)]
    stop_hook_active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct TurnBaseline {
    root: PathBuf,
    finding_identities: HashSet<String>,
    diagnostic_counts: HashMap<String, usize>,
}

pub fn run_hook() -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let event: HookInput = serde_json::from_str(&input).context("invalid Codex hook input")?;
    let output = handle_hook(event)?;
    serde_json::to_writer(io::stdout().lock(), &output)?;
    println!();
    Ok(())
}

fn handle_hook(event: HookInput) -> Result<Value> {
    let root = repository_root(&event.cwd)?;
    match event.hook_event_name.as_str() {
        "UserPromptSubmit" => capture_baseline(&event, &root),
        "PostToolUse" => post_tool_use(&event, &root),
        "Stop" => stop(&event, &root),
        _ => Ok(json!({})),
    }
}

fn capture_baseline(event: &HookInput, root: &Path) -> Result<Value> {
    let report = scan_report(root)?;
    let baseline = TurnBaseline {
        root: root.to_path_buf(),
        finding_identities: report.findings.iter().map(finding_identity).collect(),
        diagnostic_counts: report
            .diagnostics
            .iter()
            .map(|diagnostic| (diagnostic_key(diagnostic), diagnostic.count))
            .collect(),
    };
    let path = baseline_path(event)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let source = serde_json::to_string(&baseline)?;
    atomic_write(&path, &source)?;
    Ok(json!({}))
}

fn post_tool_use(event: &HookInput, root: &Path) -> Result<Value> {
    if tool_failed(&event.tool_response) {
        return Ok(json!({}));
    }
    let Some(baseline) = load_baseline(event)? else {
        return Ok(json!({}));
    };
    if baseline.root != root {
        return Ok(json!({}));
    }

    let paths = changed_paths(&event.tool_input, &event.cwd, root);
    if paths.is_empty() {
        return Ok(json!({}));
    }
    let analyses = scan_files(root, paths, &scan_options())?;
    let findings = new_file_findings(&analyses, &baseline.finding_identities);
    let diagnostics = new_file_diagnostics(&analyses, &baseline.diagnostic_counts);
    if findings.is_empty() && diagnostics.is_empty() {
        return Ok(json!({}));
    }

    let feedback = format_feedback(&findings, &diagnostics, "changed code");
    Ok(json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": feedback
        }
    }))
}

fn stop(event: &HookInput, root: &Path) -> Result<Value> {
    let Some(baseline) = load_baseline(event)? else {
        return Ok(json!({}));
    };
    if baseline.root != root {
        return Ok(json!({}));
    }

    let report = scan_report(root)?;
    let findings = report
        .findings
        .iter()
        .filter(|finding| finding.points > 0.0)
        .filter(|finding| {
            !baseline
                .finding_identities
                .contains(&finding_identity(finding))
        })
        .collect::<Vec<_>>();
    let diagnostics = report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let previous = baseline
                .diagnostic_counts
                .get(&diagnostic_key(diagnostic))
                .copied()
                .unwrap_or_default();
            (diagnostic.count > previous).then_some(DiagnosticRef {
                path: &diagnostic.path,
                count: diagnostic.count - previous,
            })
        })
        .collect::<Vec<_>>();

    if findings.is_empty() && diagnostics.is_empty() {
        let _ = fs::remove_file(baseline_path(event)?);
        return Ok(json!({}));
    }

    let feedback = format_feedback(&findings, &diagnostics, "this turn");
    if event.stop_hook_active {
        Ok(json!({ "systemMessage": feedback }))
    } else {
        Ok(json!({ "decision": "block", "reason": feedback }))
    }
}

fn scan_report(root: &Path) -> Result<ScanReport> {
    let started = Instant::now();
    let analyses = scan(root, &scan_options())?;
    Ok(build_report(root, analyses, started.elapsed()))
}

fn scan_options() -> ScanOptions {
    ScanOptions {
        include_declarations: false,
        respect_ignores: true,
        max_file_bytes: 2_000_000,
        threads: 0,
    }
}

fn new_file_findings<'a>(
    analyses: &'a [FileAnalysis],
    baseline: &HashSet<String>,
) -> Vec<&'a Finding> {
    analyses
        .iter()
        .flat_map(|analysis| analysis.findings.iter())
        .filter(|finding| finding.points > 0.0)
        .filter(|finding| !baseline.contains(&finding_identity(finding)))
        .collect()
}

fn new_file_diagnostics<'a>(
    analyses: &'a [FileAnalysis],
    baseline: &HashMap<String, usize>,
) -> Vec<DiagnosticRef<'a>> {
    analyses
        .iter()
        .filter(|analysis| analysis.parse_errors > 0)
        .filter_map(|analysis| {
            let key = diagnostic_key_parts(&analysis.display_path, "parse-error");
            let previous = baseline.get(&key).copied().unwrap_or_default();
            (analysis.parse_errors > previous).then_some(DiagnosticRef {
                path: &analysis.display_path,
                count: analysis.parse_errors - previous,
            })
        })
        .collect()
}

fn finding_identity(finding: &Finding) -> String {
    format!("{}\0{}\0{}", finding.path, finding.rule, finding.message)
}

fn diagnostic_key(diagnostic: &Diagnostic) -> String {
    diagnostic_key_parts(&diagnostic.path, diagnostic.kind)
}

fn diagnostic_key_parts(path: &str, kind: &str) -> String {
    format!("{path}\0{kind}")
}

struct DiagnosticRef<'a> {
    path: &'a str,
    count: usize,
}

trait FeedbackDiagnostic {
    fn path(&self) -> &str;
    fn count(&self) -> usize;
}

impl FeedbackDiagnostic for Diagnostic {
    fn path(&self) -> &str {
        &self.path
    }

    fn count(&self) -> usize {
        self.count
    }
}

impl FeedbackDiagnostic for DiagnosticRef<'_> {
    fn path(&self) -> &str {
        self.path
    }

    fn count(&self) -> usize {
        self.count
    }
}

impl<T> FeedbackDiagnostic for &T
where
    T: FeedbackDiagnostic + ?Sized,
{
    fn path(&self) -> &str {
        (*self).path()
    }

    fn count(&self) -> usize {
        (*self).count()
    }
}

fn format_feedback<F, D>(findings: &[F], diagnostics: &[D], scope: &str) -> String
where
    F: std::borrow::Borrow<Finding>,
    D: FeedbackDiagnostic,
{
    let total = findings.len() + diagnostics.len();
    let mut output = format!(
        "Slop detected {total} new maintainability issue{} introduced in {scope}. Fix the changed code and run the relevant tests before finishing.",
        if total == 1 { "" } else { "s" }
    );

    for diagnostic in diagnostics.iter().take(MAX_HOOK_FINDINGS) {
        output.push_str(&format!(
            "\n- {}: {} new syntax error{}",
            diagnostic.path(),
            diagnostic.count(),
            if diagnostic.count() == 1 { "" } else { "s" }
        ));
    }
    let remaining = MAX_HOOK_FINDINGS.saturating_sub(diagnostics.len());
    for finding in findings.iter().take(remaining) {
        let finding = finding.borrow();
        output.push_str(&format!(
            "\n- {}:{} [{}] {} — {}\n  Repair: {}",
            finding.path,
            finding.line,
            finding.rule,
            finding.message,
            finding.evidence,
            finding.remediation_prompt
        ));
    }
    if total > MAX_HOOK_FINDINGS {
        output.push_str(&format!(
            "\n- …and {} more; run `slop .` for the full report.",
            total - MAX_HOOK_FINDINGS
        ));
    }
    output
}

fn changed_paths(tool_input: &Value, cwd: &Path, root: &Path) -> Vec<PathBuf> {
    let mut raw = BTreeSet::new();
    collect_path_fields(tool_input, &mut raw);
    if let Some(command) = tool_input.get("command").and_then(Value::as_str) {
        collect_patch_paths(command, &mut raw);
    }

    raw.into_iter()
        .filter_map(|path| normalize_hook_path(&path, cwd, root))
        .collect()
}

fn collect_path_fields(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "path" | "file" | "file_path" | "filePath" | "target_path" | "targetPath"
                ) {
                    if let Some(path) = value.as_str() {
                        paths.insert(path.to_owned());
                    }
                }
                collect_path_fields(value, paths);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_path_fields(value, paths);
            }
        }
        _ => {}
    }
}

fn collect_patch_paths(command: &str, paths: &mut BTreeSet<String>) {
    const PREFIXES: &[&str] = &[
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    for line in command.lines() {
        if let Some(path) = PREFIXES.iter().find_map(|prefix| line.strip_prefix(prefix)) {
            let path = path.trim();
            if !path.is_empty() {
                paths.insert(path.to_owned());
            }
        }
    }
}

fn normalize_hook_path(path: &str, cwd: &Path, root: &Path) -> Option<PathBuf> {
    let path = Path::new(path);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let absolute = absolute.canonicalize().ok()?;
    absolute.starts_with(root).then_some(absolute)
}

fn tool_failed(response: &Value) -> bool {
    match response {
        Value::String(response) => {
            response.starts_with("Exit code:") && !response.starts_with("Exit code: 0")
        }
        Value::Object(response) => response
            .get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0),
        _ => false,
    }
}

fn load_baseline(event: &HookInput) -> Result<Option<TurnBaseline>> {
    let path = baseline_path(event)?;
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(&path)?;
    serde_json::from_str(&source)
        .with_context(|| format!("cannot parse hook baseline '{}'", path.display()))
        .map(Some)
}

fn baseline_path(event: &HookInput) -> Result<PathBuf> {
    let turn_id = event
        .turn_id
        .as_deref()
        .context("Codex hook input did not include turn_id")?;
    let state_root = std::env::temp_dir().join("slop-codex-hooks");
    Ok(state_root
        .join(safe_component(&event.session_id))
        .join(format!("{}.json", safe_component(turn_id))))
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}

fn repository_root(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("cannot access '{}'", path.display()))?;
    let start = if path.is_file() {
        path.parent().context("file has no parent directory")?
    } else {
        &path
    };
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return Ok(ancestor.to_path_buf());
        }
    }
    Ok(start.to_path_buf())
}

fn atomic_write(path: &Path, source: &str) -> Result<()> {
    AtomicFile::new(path, OverwriteBehavior::AllowOverwrite)
        .write(|file| -> io::Result<()> {
            file.write_all(source.as_bytes())?;
            file.sync_all()
        })
        .map_err(|error| anyhow::anyhow!(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_paths_are_extracted_without_accepting_parent_traversal() {
        let mut paths = BTreeSet::new();
        collect_patch_paths(
            "*** Begin Patch\n*** Update File: src/main.ts\n*** Move to: src/app.ts\n*** End Patch",
            &mut paths,
        );
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            vec!["src/app.ts", "src/main.ts"]
        );
    }

    #[test]
    fn existing_finding_identity_ignores_line_and_evidence_changes() {
        let first = Finding::new(
            "long-function",
            crate::model::Category::Size,
            4.0,
            ("src/app.ts".to_owned(), 10),
            ("`loadApp` is too long", "61 lines"),
        );
        let improved = Finding::new(
            "long-function",
            crate::model::Category::Size,
            4.0,
            ("src/app.ts".to_owned(), 14),
            ("`loadApp` is too long", "60 lines"),
        );
        assert_eq!(finding_identity(&first), finding_identity(&improved));
    }
}
