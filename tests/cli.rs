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
