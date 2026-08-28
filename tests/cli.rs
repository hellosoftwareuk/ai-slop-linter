use std::{fs, process::Command};

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
