use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn fix_flag_is_opt_in_and_json_reports_post_fix_results() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("sample.ts");
    let original = "let stable = 1;\nconst output = ready ? true : false;\n";
    fs::write(&path, original)?;

    let inspect = Command::new(env!("CARGO_BIN_EXE_slop"))
        .arg(directory.path())
        .args(["--format", "json"])
        .output()?;
    assert!(
        inspect.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&inspect.stdout)?;
    assert_eq!(report["fixes"]["requested"], false);
    assert!(report["findings"]
        .as_array()
        .expect("findings should be an array")
        .iter()
        .any(|finding| finding["fixable"] == true));
    assert_eq!(fs::read_to_string(&path)?, original);

    let fix = Command::new(env!("CARGO_BIN_EXE_slop"))
        .arg(directory.path())
        .args(["--fix", "--format", "json", "--fail-above", "0"])
        .output()?;
    assert!(
        fix.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&fix.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&fix.stdout)?;
    assert_eq!(report["fixes"]["requested"], true);
    assert_eq!(report["fixes"]["applied"], 2);
    assert_eq!(report["fixes"]["files_changed"], 1);
    assert!(report["findings"]
        .as_array()
        .expect("findings should be an array")
        .iter()
        .all(|finding| finding["fixable"] == false));
    assert_eq!(
        fs::read_to_string(path)?,
        "const stable = 1;\nconst output = (!!(ready));\n"
    );
    Ok(())
}

#[test]
fn codex_hook_reports_only_slop_introduced_during_the_turn() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join(".git"))?;
    let path = directory.path().join("sample.ts");
    fs::write(&path, "const stable = 1;\n")?;
    let session = format!(
        "slop-test-{}",
        directory
            .path()
            .file_name()
            .expect("temporary directory should have a name")
            .to_string_lossy()
    );
    let turn = "turn-1";

    let baseline = run_codex_hook(serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "cwd": directory.path(),
        "session_id": session,
        "turn_id": turn
    }))?;
    assert_eq!(baseline, serde_json::json!({}));

    fs::write(&path, "let stable = 1;\n")?;
    let post_edit = run_codex_hook(serde_json::json!({
        "hook_event_name": "PostToolUse",
        "cwd": directory.path(),
        "session_id": session,
        "turn_id": turn,
        "tool_input": {
            "command": "*** Begin Patch\n*** Update File: sample.ts\n*** End Patch"
        },
        "tool_response": "Exit code: 0\nSuccess"
    }))?;
    let context = post_edit["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("post-edit hook should return model context");
    assert!(context.contains("prefer-const"), "context: {context}");

    let stop = run_codex_hook(serde_json::json!({
        "hook_event_name": "Stop",
        "cwd": directory.path(),
        "session_id": session,
        "turn_id": turn,
        "stop_hook_active": false
    }))?;
    assert_eq!(stop["decision"], "block");
    assert!(stop["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("prefer-const")));

    fs::write(path, "const stable = 1;\n")?;
    let clean_stop = run_codex_hook(serde_json::json!({
        "hook_event_name": "Stop",
        "cwd": directory.path(),
        "session_id": session,
        "turn_id": turn,
        "stop_hook_active": true
    }))?;
    assert_eq!(clean_stop, serde_json::json!({}));
    Ok(())
}

#[test]
fn diff_reports_only_findings_introduced_after_head() -> anyhow::Result<()> {
    let directory = initialized_repository("const customerCount = 1;\n")?;
    let path = directory.path().join("sample.ts");
    fs::write(&path, "let customerCount = 1;\n")?;

    let output = slop_diff(directory.path(), &["--format", "json"])?;
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["comparison"], "working_tree");
    assert_eq!(report["base"], "HEAD");
    assert_eq!(report["files_changed"], 1);
    assert!(report["new_debt_points"]
        .as_f64()
        .is_some_and(|value| value > 0.0));
    assert!(report["findings"]
        .as_array()
        .expect("findings should be an array")
        .iter()
        .any(|finding| finding["rule"] == "prefer-const"));

    fs::write(&path, "const customerCount = 1;\n\n")?;
    let clean = slop_diff(directory.path(), &["--format", "json"])?;
    assert!(
        clean.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&clean.stdout)?;
    assert_eq!(report["files_changed"], 1);
    assert_eq!(report["findings"].as_array().map(Vec::len), Some(0));
    assert_eq!(report["diagnostics"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[test]
fn diff_keeps_preexisting_debt_quiet_and_scans_untracked_files() -> anyhow::Result<()> {
    let directory = initialized_repository("let existingCustomerCount = 1;\n")?;
    fs::write(
        directory.path().join("sample.ts"),
        "let existingCustomerCount = 1;\n\n",
    )?;

    let unchanged_debt = slop_diff(directory.path(), &["--format", "json"])?;
    assert!(unchanged_debt.status.success());
    let report: serde_json::Value = serde_json::from_slice(&unchanged_debt.stdout)?;
    assert_eq!(report["findings"].as_array().map(Vec::len), Some(0));

    fs::create_dir(directory.path().join("src"))?;
    fs::write(
        directory.path().join("src/tracked.ts"),
        "const trackedCustomerCount = 2;\n",
    )?;
    git(directory.path(), &["add", "src/tracked.ts"])?;
    git(
        directory.path(),
        &["commit", "--quiet", "-m", "add source folder"],
    )?;
    fs::write(
        directory.path().join("src/new.ts"),
        "let untrackedCustomerCount = 2;\n",
    )?;
    let untracked = slop_diff(&directory.path().join("src"), &["--format", "github"])?;
    assert_eq!(untracked.status.code(), Some(2));
    let output = String::from_utf8(untracked.stdout)?;
    assert!(output.contains("::warning file=src/new.ts,line=1,title=Slop [prefer-const]::"));
    Ok(())
}

#[test]
fn staged_diff_reads_the_index_and_ignores_unstaged_repairs() -> anyhow::Result<()> {
    let directory = initialized_repository("const invoiceCount = 1;\n")?;
    let path = directory.path().join("sample.ts");
    fs::write(&path, "let invoiceCount = 1;\n")?;
    git(directory.path(), &["add", "sample.ts"])?;
    fs::write(&path, "const invoiceCount = 1;\n")?;

    let staged = slop_diff(directory.path(), &["--staged", "--format", "json"])?;
    assert_eq!(staged.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&staged.stdout)?;
    assert_eq!(report["comparison"], "staged");
    assert_eq!(report["files_changed"], 1);
    assert!(report["findings"]
        .as_array()
        .expect("findings should be an array")
        .iter()
        .any(|finding| finding["rule"] == "prefer-const"));

    let working_tree = slop_diff(directory.path(), &["--format", "json"])?;
    assert!(working_tree.status.success());
    Ok(())
}

#[test]
fn diff_rejects_an_unknown_explicit_base() -> anyhow::Result<()> {
    let directory = initialized_repository("const invoiceCount = 1;\n")?;
    let output = slop_diff(directory.path(), &["--base", "missing-revision"])?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("cannot resolve base revision"));
    Ok(())
}

fn initialized_repository(source: &str) -> anyhow::Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(
        directory.path(),
        &["config", "user.email", "slop@example.test"],
    )?;
    git(directory.path(), &["config", "user.name", "Slop Test"])?;
    fs::write(directory.path().join("sample.ts"), source)?;
    git(directory.path(), &["add", "sample.ts"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "baseline"])?;
    Ok(directory)
}

fn git(directory: &std::path::Path, arguments: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn slop_diff(
    directory: &std::path::Path,
    arguments: &[&str],
) -> std::io::Result<std::process::Output> {
    Command::new(env!("CARGO_BIN_EXE_slop"))
        .arg("diff")
        .arg(directory)
        .args(arguments)
        .output()
}

fn run_codex_hook(input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_slop"))
        .args(["codex", "hook"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("hook stdin should be piped")
        .write_all(serde_json::to_string(&input)?.as_bytes())?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}
