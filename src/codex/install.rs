use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use super::{atomic_write, repository_root};

const HOOK_COMMAND: &str = "slop codex hook";

pub fn install(path: &Path) -> Result<PathBuf> {
    let root = repository_root(path)?;
    let codex_dir = root.join(".codex");
    fs::create_dir_all(&codex_dir)
        .with_context(|| format!("cannot create '{}'", codex_dir.display()))?;
    let hooks_path = codex_dir.join("hooks.json");
    let mut document = read_hooks(&hooks_path)?;

    let root_object = document
        .as_object_mut()
        .context("existing .codex/hooks.json must contain a JSON object")?;
    let hooks = object_entry(root_object, "hooks")?;
    add_hook(hooks, "UserPromptSubmit", None, "Capturing Slop baseline")?;
    add_hook(
        hooks,
        "PostToolUse",
        Some("Edit|Write|apply_patch"),
        "Scanning changed code with Slop",
    )?;
    add_hook(hooks, "Stop", None, "Checking this turn for new slop")?;

    let mut rendered = serde_json::to_string_pretty(&document)?;
    rendered.push('\n');
    atomic_write(&hooks_path, &rendered)
        .with_context(|| format!("cannot write '{}'", hooks_path.display()))?;
    Ok(hooks_path)
}

fn read_hooks(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let source =
        fs::read_to_string(path).with_context(|| format!("cannot read '{}'", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("cannot parse '{}'", path.display()))
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    object
        .entry(key.to_owned())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("'{key}' in .codex/hooks.json must be an object"))
}

fn add_hook(
    hooks: &mut Map<String, Value>,
    event: &str,
    matcher: Option<&str>,
    status_message: &str,
) -> Result<()> {
    let groups = hooks
        .entry(event.to_owned())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .with_context(|| format!("hooks.{event} must be an array"))?;
    if command_is_installed(groups) {
        return Ok(());
    }

    let mut group = json!({
        "hooks": [{
            "type": "command",
            "command": HOOK_COMMAND,
            "timeout": 10,
            "statusMessage": status_message
        }]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = Value::String(matcher.to_owned());
    }
    groups.push(group);
    Ok(())
}

fn command_is_installed(groups: &[Value]) -> bool {
    for group in groups {
        let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
            continue;
        };
        for handler in handlers {
            let is_command = handler.get("type").and_then(Value::as_str) == Some("command");
            let is_slop = handler.get("command").and_then(Value::as_str) == Some(HOOK_COMMAND);
            if is_command && is_slop {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_install_is_idempotent() -> Result<()> {
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join(".git"))?;
        fs::create_dir(directory.path().join(".codex"))?;
        fs::write(
            directory.path().join(".codex/hooks.json"),
            serde_json::to_vec(&json!({
                "metadata": "preserve-me",
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "echo existing"}]
                    }]
                }
            }))?,
        )?;
        let path = install(directory.path())?;
        install(directory.path())?;
        let document: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(document["metadata"], "preserve-me");
        assert_eq!(document["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        for event in ["UserPromptSubmit", "PostToolUse", "Stop"] {
            assert_eq!(document["hooks"][event].as_array().map(Vec::len), Some(1));
        }
        assert_eq!(
            document["hooks"]["PostToolUse"][0]["matcher"],
            "Edit|Write|apply_patch"
        );
        Ok(())
    }
}
